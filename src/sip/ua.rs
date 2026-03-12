use crate::core::{Account, CallState};
use crate::sip::{SipRequest, Method, SipHeaderAccess, SipMessage};
use crate::sip::transport::SipTransport;
use crate::sip::auth::{calculate_digest_response, calculate_digest_response_qop};
use crate::media::sdp::{SdpSession, SdpMediaDescription};
use anyhow::{Result, anyhow};
use tokio::time::{timeout, Duration};
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use uuid::Uuid;
use tracing::{info, warn, error};
use tokio::sync::mpsc;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Call {
    pub id: String,
    pub state: CallState,
    pub remote_uri: String,
    pub local_tag: String,
    pub remote_tag: Option<String>,
    pub remote_contact: Option<String>,
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
    subscriptions: HashMap<String, mpsc::Sender<SipMessage>>,
}

fn extract_tag(header: &str) -> Option<String> {
    header.split(';').find(|p| p.trim().starts_with("tag="))
        .and_then(|p| p.split_once('='))
        .map(|(_, v)| v.trim().to_string())
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
            subscriptions: HashMap::new(),
        }
    }

    pub fn dispatch(&mut self, msg: SipMessage) {
        let call_id = match &msg {
            SipMessage::Request(req) => req.call_id().cloned(),
            SipMessage::Response(res) => res.call_id().cloned(),
        };

        if let Some(cid) = call_id {
            if let Some(tx) = self.subscriptions.get(&cid) {
                let _ = tx.try_send(msg);
            }
        }
    }

    pub fn subscribe(&mut self, call_id: String) -> mpsc::Receiver<SipMessage> {
        let (tx, rx) = mpsc::channel(10);
        self.subscriptions.insert(call_id, tx);
        rx
    }

    pub fn unsubscribe(&mut self, call_id: &str) {
        self.subscriptions.remove(call_id);
    }

    fn get_public_local_addr(&self, target: SocketAddr) -> String {
        let socket = StdUdpSocket::bind("0.0.0.0:0").ok();
        let local_ip = socket.and_then(|s| {
            s.connect(target).ok()?;
            s.local_addr().ok()
        }).map(|a| a.ip().to_string()).unwrap_or_else(|| "127.0.0.1".to_string());

        let port = self.transport.local_addr().map(|a| a.port()).unwrap_or(5060);
        format!("{}:{}", local_ip, port)
    }

    fn new_branch(&self) -> String {
        format!("z9hG4bK-{}", Uuid::new_v4())
    }

    pub async fn register(&mut self, server_addr: SocketAddr) -> Result<()> {
        self.reg_state = RegistrationState::Registering;
        info!("Registration started for {}", self.account.username);

        let uri = format!("sip:{}", self.account.domain);
        let local_addr = self.get_public_local_addr(server_addr);
        let proto = self.transport.protocol().to_string();
        let mut rx = self.subscribe(self.call_id.clone());
        let cid = self.call_id.clone();

        self.cseq += 1;
        let req = SipRequest::new(Method::Register, &uri)
            .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
            .with_header("To", &format!("<sip:{}@{}>", self.account.username, self.account.domain))
            .with_header("Call-ID", &cid)
            .with_header("CSeq", &format!("{} REGISTER", self.cseq))
            .with_header("Contact", &format!("<sip:{}@{}>", self.account.username, local_addr))
            .with_header("Max-Forwards", "70")
            .with_header("User-Agent", "Softphone/0.1.0")
            .with_header("Expires", "3600");

        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;

        let res = match timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(SipMessage::Response(res))) => res,
            _ => {
                self.reg_state = RegistrationState::Failed("Timeout waiting for response".to_string());
                self.unsubscribe(&cid);
                return Err(anyhow!("Timeout waiting for response"));
            }
        };

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
                    .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
                    .with_header("CSeq", &format!("{} REGISTER", self.cseq))
                    .with_header(if res.status_code == 401 { "Authorization" } else { "Proxy-Authorization" }, &auth_val);

                self.transport.send_to(auth_req.to_string().as_bytes(), server_addr).await?;

                let final_res = match timeout(Duration::from_secs(5), rx.recv()).await {
                    Ok(Some(SipMessage::Response(res))) => res,
                    _ => {
                        self.reg_state = RegistrationState::Failed("Timeout waiting for auth response".to_string());
                        self.unsubscribe(&cid);
                        return Err(anyhow!("Timeout waiting for auth response"));
                    }
                };

                if final_res.status_code == 200 {
                    self.reg_state = RegistrationState::Registered;
                    info!("Registered successfully (with auth)");
                } else {
                    self.reg_state = RegistrationState::Failed(format!("Status {}", final_res.status_code));
                    error!("Registration failed with status {}", final_res.status_code);
                }
            }
            _ => {
                self.reg_state = RegistrationState::Failed(format!("Status {}", res.status_code));
                warn!("Registration failed with status {}", res.status_code);
            }
        }

        self.unsubscribe(&cid);
        Ok(())
    }

    pub async fn invite(&mut self, remote_uri: &str, server_addr: SocketAddr) -> Result<()> {
        let call_id = Uuid::new_v4().to_string();
        let local_tag = Uuid::new_v4().to_string()[..8].to_string();
        self.cseq += 1;
        let local_addr = self.get_public_local_addr(server_addr);
        let proto = self.transport.protocol().to_string();
        let mut rx = self.subscribe(call_id.clone());
        let cid = call_id.clone();

        let mut sdp = SdpSession::new(&self.account.username, "CallSession", &local_addr.split(':').next().unwrap_or("0.0.0.0"));
        sdp.add_media(SdpMediaDescription {
            media_type: "audio".to_string(),
            port: 4000,
            transport: "RTP/AVP".to_string(),
            formats: vec!["0".to_string(), "8".to_string()],
            attributes: vec!["rtpmap:0 PCMU/8000".to_string(), "rtpmap:8 PCMA/8000".to_string()],
        });
        let sdp_str = sdp.to_string();

        let mut req = SipRequest::new(Method::Invite, remote_uri)
            .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, local_tag))
            .with_header("To", &format!("<{}>", remote_uri))
            .with_header("Call-ID", &cid)
            .with_header("CSeq", &format!("{} INVITE", self.cseq))
            .with_header("Contact", &format!("<sip:{}@{}>", self.account.username, local_addr))
            .with_header("Content-Type", "application/sdp")
            .with_header("User-Agent", "Softphone/0.1.0")
            .with_header("Max-Forwards", "70");

        req.body = sdp_str.into_bytes();

        info!("Sending INVITE to {} for {}", server_addr, remote_uri);
        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;

        self.active_calls.push(Call {
            id: cid.clone(),
            state: CallState::Calling,
            remote_uri: remote_uri.to_string(),
            local_tag,
            remote_tag: None,
            remote_contact: None,
        });

        let mut retry_count = 0;
        loop {
            let msg = match timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(msg)) => msg,
                _ => {
                    warn!("Timeout waiting for INVITE response");
                    break;
                }
            };

            if let SipMessage::Response(res) = msg {
                info!("Received {} {} for INVITE", res.status_code, res.reason);

                let r_tag = res.get_header("To").and_then(|h| extract_tag(h));
                let r_contact = res.get_header("Contact").map(|h| h.trim_matches(|c| c == '<' || c == '>').to_string());

                if let Some(pos) = self.active_calls.iter().position(|c| c.id == cid) {
                    if let Some(tag) = r_tag.clone() { self.active_calls[pos].remote_tag = Some(tag); }
                    if let Some(contact) = r_contact.clone() { self.active_calls[pos].remote_contact = Some(contact); }
                }

                if res.status_code == 100 || res.status_code == 180 || res.status_code == 183 {
                    if let Some(pos) = self.active_calls.iter().position(|c| c.id == cid) {
                        if res.status_code == 180 || res.status_code == 183 {
                            self.active_calls[pos].state = CallState::Ringing;
                        }
                    }
                    continue;
                }
                if res.status_code == 200 {
                    if let Some(pos) = self.active_calls.iter().position(|c| c.id == cid) {
                        self.active_calls[pos].state = CallState::Connected;
                        let call = self.active_calls[pos].clone();

                        let ack_uri = call.remote_contact.as_ref().unwrap_or(&call.remote_uri);
                        let ack = SipRequest::new(Method::Ack, ack_uri)
                            .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
                            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, call.local_tag))
                            .with_header("To", &format!("<{}>;tag={}", call.remote_uri, call.remote_tag.as_deref().unwrap_or("")))
                            .with_header("Call-ID", &call.id)
                            .with_header("CSeq", &format!("{} ACK", self.cseq))
                            .with_header("Max-Forwards", "70");

                        info!("Sending ACK to {}", server_addr);
                        self.transport.send_to(ack.to_string().as_bytes(), server_addr).await?;
                    }
                    break;
                }
                if (res.status_code == 401 || res.status_code == 407) && retry_count < 3 {
                    retry_count += 1;
                    let auth_header_name = if res.status_code == 401 { "WWW-Authenticate" } else { "Proxy-Authenticate" };
                    if let Some(authenticate) = res.get_header(auth_header_name) {
                        let realm = authenticate.split("realm=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");
                        let nonce = authenticate.split("nonce=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");
                        let qop = authenticate.split("qop=\"").nth(1).and_then(|s| s.split('\"').next());

                        let password = self.account.password.as_ref().ok_or_else(|| anyhow!("Password required"))?;
                        let method_str = Method::Invite.to_string();

                        let mut auth_val = format!("Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\"",
                            self.account.username, realm, nonce, remote_uri);

                        if let Some(_q) = qop.filter(|&q| q.contains("auth")) {
                            let cnonce = Uuid::new_v4().to_string()[..8].to_string();
                            let nc = "00000001";
                            let response = calculate_digest_response_qop(&self.account.username, password, realm, nonce, &cnonce, nc, "auth", &method_str, remote_uri);
                            auth_val.push_str(&format!(", qop=\"auth\", nc={}, cnonce=\"{}\", response=\"{}\"", nc, cnonce, response));
                        } else {
                            let response = calculate_digest_response(&self.account.username, password, realm, nonce, &method_str, remote_uri);
                            auth_val.push_str(&format!(", response=\"{}\"", response));
                        }

                        self.cseq += 1;
                        let mut auth_req = req.clone();
                        auth_req.set_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()));
                        auth_req.set_header("CSeq", &format!("{} INVITE", self.cseq));
                        auth_req.set_header(if res.status_code == 401 { "Authorization" } else { "Proxy-Authorization" }, &auth_val);

                        info!("Retrying INVITE with auth (retry {}) to {}", retry_count, server_addr);
                        self.transport.send_to(auth_req.to_string().as_bytes(), server_addr).await?;
                        continue;
                    }
                }
                if res.status_code >= 400 {
                    error!("INVITE failed with status {}: {}", res.status_code, res.reason);
                    if let Some(pos) = self.active_calls.iter().position(|c| c.id == cid) {
                        self.active_calls.remove(pos);
                    }
                    break;
                }
            }
        }

        self.unsubscribe(&cid);
        Ok(())
    }

    pub async fn hangup(&mut self, call_id: String, server_addr: SocketAddr) -> Result<()> {
        if let Some(pos) = self.active_calls.iter().position(|c| c.id == call_id) {
            let call = self.active_calls[pos].clone();
            self.cseq += 1;
            let local_addr = self.get_public_local_addr(server_addr);
            let proto = self.transport.protocol().to_string();

            let bye_uri = call.remote_contact.as_ref().unwrap_or(&call.remote_uri);
            let req = SipRequest::new(Method::Bye, bye_uri)
                .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
                .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, call.local_tag))
                .with_header("To", &format!("<{}>;tag={}", call.remote_uri, call.remote_tag.as_deref().unwrap_or("")))
                .with_header("Call-ID", &call.id)
                .with_header("CSeq", &format!("{} BYE", self.cseq))
                .with_header("User-Agent", "Softphone/0.1.0")
                .with_header("Max-Forwards", "70");

            self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
            self.active_calls.remove(pos);
            info!("Hung up call {}", call_id);
        }
        Ok(())
    }
}
