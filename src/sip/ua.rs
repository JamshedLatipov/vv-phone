use crate::core::{Account, CallState};
use crate::sip::{SipRequest, Method, SipHeaderAccess, SipMessage};
use crate::sip::transport::SipTransport;
use crate::sip::auth::{calculate_digest_response, calculate_digest_response_qop};
use crate::media::sdp::{SdpSession, SdpMediaDescription};
use anyhow::{Result, anyhow};
use tokio::time::{timeout, Duration};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;
use tracing::{info, warn, error};

#[derive(Debug, Clone)]
pub struct Call {
    pub id: String,
    pub state: CallState,
    pub remote_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationState {
    Unregistered,
    Registering,
    Registered,
    Failed(String),
}

pub struct UserAgent {
    pub account: Account,
    pub transport: Arc<dyn SipTransport>,
    pub call_id: String,
    pub cseq: u32,
    pub active_calls: Vec<Call>,
    pub reg_state: RegistrationState,
}

impl UserAgent {
    pub fn new(account: Account, transport: Arc<dyn SipTransport>) -> Self {
        Self {
            account,
            transport,
            call_id: Uuid::new_v4().to_string(),
            cseq: 1,
            active_calls: Vec::new(),
            reg_state: RegistrationState::Unregistered,
        }
    }

    pub async fn register(&mut self, server_addr: SocketAddr) -> Result<()> {
        self.reg_state = RegistrationState::Registering;
        info!("Registration started for {}", self.account.username);

        let uri = format!("sip:{}", self.account.domain);
        let local_addr = self.transport.local_addr()?.to_string();
        let proto = self.transport.protocol();

        let req = SipRequest::new(Method::Register, &uri)
            .with_header("Via", &format!("SIP/2.0/{} {};branch=z9hG4bK-register-{}", proto, local_addr, Uuid::new_v4()))
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
            .with_header("To", &format!("<sip:{}@{}>", self.account.username, self.account.domain))
            .with_header("Call-ID", &self.call_id)
            .with_header("CSeq", &format!("{} REGISTER", self.cseq))
            .with_header("Contact", &format!("<sip:{}@{}>", self.account.username, local_addr))
            .with_header("Max-Forwards", "70")
            .with_header("Expires", "3600");

        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;

        let mut buf = [0u8; 4096];
        let (len, _addr) = match timeout(Duration::from_secs(5), self.transport.recv_from(&mut buf)).await {
            Ok(res) => res?,
            Err(_) => {
                self.reg_state = RegistrationState::Failed("Timeout waiting for response".to_string());
                return Err(anyhow!("Timeout waiting for response"));
            }
        };
        let resp_str = String::from_utf8_lossy(&buf[..len]);

        if let Some(SipMessage::Response(res)) = SipMessage::parse(&resp_str) {
            match res.status_code {
                200 => {
                    self.reg_state = RegistrationState::Registered;
                    info!("Registered successfully");
                }
                401 | 407 => {
                    let auth_header_name = if res.status_code == 401 { "WWW-Authenticate" } else { "Proxy-Authenticate" };
                    let authenticate = res.get_header(auth_header_name).ok_or_else(|| anyhow!("Missing Authentication header"))?;

                    let realm = authenticate.split("realm=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");
                    let nonce = authenticate.split("nonce=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");
                    let qop = authenticate.split("qop=\"").nth(1).and_then(|s| s.split('\"').next());

                    let password = self.account.password.as_ref().ok_or_else(|| anyhow!("Password required"))?;
                    let method_str = Method::Register.to_string();

                    let mut auth_val = format!("Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\"",
                        self.account.username, realm, nonce, uri);

                    if let Some(_q) = qop.filter(|&q| q.contains("auth")) {
                        let cnonce = Uuid::new_v4().to_string()[..8].to_string();
                        let nc = "00000001";
                        let response = calculate_digest_response_qop(&self.account.username, password, realm, nonce, &cnonce, nc, "auth", &method_str, &uri);
                        auth_val.push_str(&format!(", qop=\"auth\", nc={}, cnonce=\"{}\", response=\"{}\"", nc, cnonce, response));
                    } else {
                        let response = calculate_digest_response(&self.account.username, password, realm, nonce, &method_str, &uri);
                        auth_val.push_str(&format!(", response=\"{}\"", response));
                    }

                    self.cseq += 1;
                    let auth_req = req.clone()
                        .with_header("CSeq", &format!("{} REGISTER", self.cseq))
                        .with_header(if res.status_code == 401 { "Authorization" } else { "Proxy-Authorization" }, &auth_val);

                    self.transport.send_to(auth_req.to_string().as_bytes(), server_addr).await?;

                    let (len, _addr) = match timeout(Duration::from_secs(5), self.transport.recv_from(&mut buf)).await {
                        Ok(res) => res?,
                        Err(_) => {
                            self.reg_state = RegistrationState::Failed("Timeout waiting for auth response".to_string());
                            return Err(anyhow!("Timeout waiting for auth response"));
                        }
                    };
                    let resp_str = String::from_utf8_lossy(&buf[..len]);
                    if let Some(SipMessage::Response(final_res)) = SipMessage::parse(&resp_str) {
                        if final_res.status_code == 200 {
                            self.reg_state = RegistrationState::Registered;
                            info!("Registered successfully (with auth)");
                        } else {
                            self.reg_state = RegistrationState::Failed(format!("Status {}", final_res.status_code));
                            error!("Registration failed with status {}", final_res.status_code);
                        }
                    }
                }
                _ => {
                    self.reg_state = RegistrationState::Failed(format!("Status {}", res.status_code));
                    warn!("Registration failed with status {}", res.status_code);
                }
            }
        }

        Ok(())
    }

    pub async fn invite(&mut self, remote_uri: &str, server_addr: SocketAddr) -> Result<()> {
        let call_id = Uuid::new_v4().to_string();
        self.cseq += 1;
        let local_addr = self.transport.local_addr()?.to_string();
        let proto = self.transport.protocol();

        let mut sdp = SdpSession::new(&self.account.username, "CallSession", &local_addr.split(':').next().unwrap_or("0.0.0.0"));
        sdp.add_media(SdpMediaDescription {
            media_type: "audio".to_string(),
            port: 4000,
            transport: "RTP/AVP".to_string(),
            formats: vec!["0".to_string(), "8".to_string()],
            attributes: vec!["rtpmap:0 PCMU/8000".to_string(), "rtpmap:8 PCMA/8000".to_string()],
        });
        let sdp_str = sdp.to_string();

        let req = SipRequest::new(Method::Invite, remote_uri)
            .with_header("Via", &format!("SIP/2.0/{} {};branch=z9hG4bK-invite-{}", proto, local_addr, Uuid::new_v4()))
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
            .with_header("To", &format!("<{}>", remote_uri))
            .with_header("Call-ID", &call_id)
            .with_header("CSeq", &format!("{} INVITE", self.cseq))
            .with_header("Contact", &format!("<sip:{}@{}>", self.account.username, local_addr))
            .with_header("Content-Type", "application/sdp")
            .with_header("Max-Forwards", "70");

        let mut req = req;
        req.body = sdp_str.into_bytes();

        self.active_calls.push(Call {
            id: call_id,
            state: CallState::Calling,
            remote_uri: remote_uri.to_string(),
        });

        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
        Ok(())
    }

    pub async fn hangup(&mut self, call_id: String, server_addr: SocketAddr) -> Result<()> {
        if let Some(pos) = self.active_calls.iter().position(|c| c.id == call_id) {
            let call = &self.active_calls[pos];
            self.cseq += 1;
            let local_addr = self.transport.local_addr()?.to_string();
            let proto = self.transport.protocol();

            let req = SipRequest::new(Method::Bye, &call.remote_uri)
                .with_header("Via", &format!("SIP/2.0/{} {};branch=z9hG4bK-bye-{}", proto, local_addr, Uuid::new_v4()))
                .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
                .with_header("To", &format!("<{}>", call.remote_uri))
                .with_header("Call-ID", &call.id)
                .with_header("CSeq", &format!("{} BYE", self.cseq))
                .with_header("Max-Forwards", "70");

            self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
            self.active_calls.remove(pos);
            info!("Hung up call {}", call_id);
        }
        Ok(())
    }
}
