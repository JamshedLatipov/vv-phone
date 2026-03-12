use crate::core::{Account, CallState};
use crate::sip::{SipRequest, Method, SipHeaderAccess, SipMessage};
use crate::sip::transport::SipTransport;
use crate::sip::auth::{calculate_digest_response, calculate_digest_response_qop};
use crate::media::sdp::{SdpSession, SdpMediaDescription};
use anyhow::{Result, anyhow};
use tokio::time::{timeout, Duration};
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::{Arc, Mutex as StdMutex};
use uuid::Uuid;
use tracing::{info, warn, error, debug};
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
    pub remote_rtp_addr: Option<SocketAddr>,
    pub local_rtp_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationState {
    Unregistered,
    Registering,
    Registered,
    Failed(String),
}

#[derive(Clone)]
pub struct SipDispatcher {
    subscriptions: Arc<StdMutex<HashMap<String, mpsc::Sender<SipMessage>>>>,
}

impl SipDispatcher {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub fn dispatch(&self, msg: SipMessage) {
        let call_id = match &msg {
            SipMessage::Request(req) => req.call_id().cloned(),
            SipMessage::Response(res) => res.call_id().cloned(),
        };

        if let Some(cid) = call_id {
            let subs = self.subscriptions.lock().unwrap();
            if let Some(tx) = subs.get(&cid) {
                if let Err(_) = tx.try_send(msg) {
                    debug!("Dispatcher: Channel full or closed for Call-ID {}", cid);
                }
            } else {
                debug!("Dispatcher: No subscriber for Call-ID {}", cid);
            }
        }
    }

    pub fn subscribe(&self, call_id: String) -> mpsc::Receiver<SipMessage> {
        let (tx, rx) = mpsc::channel(100);
        let mut subs = self.subscriptions.lock().unwrap();
        subs.insert(call_id, tx);
        rx
    }

    pub fn unsubscribe(&self, call_id: &str) {
        let mut subs = self.subscriptions.lock().unwrap();
        subs.remove(call_id);
    }
}

pub struct UserAgent {
    pub account: Account,
    pub transport: Arc<dyn SipTransport>,
    pub call_id: String,
    pub cseq: u32,
    pub active_calls: Vec<Call>,
    pub reg_state: RegistrationState,
    pub dispatcher: SipDispatcher,
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
            dispatcher: SipDispatcher::new(),
        }
    }

    fn new_branch(&self) -> String {
        format!("z9hG4bK{}", Uuid::new_v4().to_string()[..8].to_string())
    }

    fn get_public_local_addr(&self, target_server: SocketAddr) -> String {
        let socket = match StdUdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(_) => return "127.0.0.1:5060".to_string(),
        };
        if socket.connect(target_server).is_err() {
            return "127.0.0.1:5060".to_string();
        }
        socket.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| "127.0.0.1:5060".to_string())
    }

    pub async fn register(&mut self, server_addr: SocketAddr) -> Result<()> {
        self.reg_state = RegistrationState::Registering;
        info!("Registration started for {}", self.account.username);

        let local_addr = self.get_public_local_addr(server_addr);
        let proto = self.transport.protocol().to_string();
        let cid = Uuid::new_v4().to_string();
        let mut rx = self.dispatcher.subscribe(cid.clone());

        let mut req = SipRequest::new(Method::Register, &format!("sip:{}", self.account.domain))
            .with_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()))
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4().to_string()[..8].to_string()))
            .with_header("To", &format!("<sip:{}@{}>", self.account.username, self.account.domain))
            .with_header("Call-ID", &cid)
            .with_header("CSeq", &format!("{} REGISTER", self.cseq))
            .with_header("Contact", &format!("<sip:{}@{}>", self.account.username, local_addr))
            .with_header("User-Agent", "Softphone/0.1.0")
            .with_header("Max-Forwards", "70");

        info!("Sending REGISTER to {} (Call-ID: {})", server_addr, cid);
        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;

        let mut retry_count = 0;
        loop {
            let msg = match timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(msg)) => msg,
                _ => {
                    self.reg_state = RegistrationState::Failed("Timeout".to_string());
                    break;
                }
            };

            if let SipMessage::Response(res) = msg {
                info!("Received {} {} for REGISTER", res.status_code, res.reason);
                if res.status_code == 200 {
                    self.reg_state = RegistrationState::Registered;
                    info!("Registered successfully");
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
                        let method_str = Method::Register.to_string();
                        let uri = format!("sip:{}", self.account.domain);

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
                        req.set_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()));
                        req.set_header("CSeq", &format!("{} REGISTER", self.cseq));
                        req.set_header(if res.status_code == 401 { "Authorization" } else { "Proxy-Authorization" }, &auth_val);

                        info!("Retrying REGISTER with auth (retry {}) to {}", retry_count, server_addr);
                        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
                        continue;
                    }
                }
                if res.status_code >= 400 {
                    self.reg_state = RegistrationState::Failed(format!("Status {}", res.status_code));
                    error!("Registration failed with status {}: {}", res.status_code, res.reason);
                    break;
                }
            }
        }

        self.dispatcher.unsubscribe(&cid);
        Ok(())
    }

    pub async fn invite(&mut self, remote_uri: &str, server_addr: SocketAddr) -> Result<()> {
        let call_id = Uuid::new_v4().to_string();
        let local_tag = Uuid::new_v4().to_string()[..8].to_string();
        self.cseq += 1;
        let local_addr = self.get_public_local_addr(server_addr);
        let proto = self.transport.protocol().to_string();
        let cid = call_id.clone();
        let mut rx = self.dispatcher.subscribe(cid.clone());

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

        info!("Sending INVITE to {} for {} (Call-ID: {})", server_addr, remote_uri, cid);
        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;

        self.active_calls.push(Call {
            id: cid.clone(),
            state: CallState::Calling,
            remote_uri: remote_uri.to_string(),
            local_tag,
            remote_tag: None,
            remote_contact: None,
            remote_rtp_addr: None,
            local_rtp_port: Some(4000),
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

                        // Parse SDP for remote RTP address
                        let body_str = String::from_utf8_lossy(&res.body);
                        if let Some(remote_sdp) = SdpSession::parse(&body_str) {
                            if let Some(media) = remote_sdp.media_descriptions.iter().find(|m| m.media_type == "audio") {
                                let c_info = if !remote_sdp.connection_info.is_empty() {
                                    &remote_sdp.connection_info
                                } else {
                                    ""
                                };
                                let ip = c_info.split_whitespace().last().unwrap_or("0.0.0.0");
                                if let Ok(addr) = format!("{}:{}", ip, media.port).parse::<SocketAddr>() {
                                    self.active_calls[pos].remote_rtp_addr = Some(addr);
                                    info!("Negotiated RTP address: {}", addr);
                                }
                            }
                        }

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
                        req.set_header("Via", &format!("SIP/2.0/{} {};branch={}", proto, local_addr, self.new_branch()));
                        req.set_header("CSeq", &format!("{} INVITE", self.cseq));
                        req.set_header(if res.status_code == 401 { "Authorization" } else { "Proxy-Authorization" }, &auth_val);

                        info!("Retrying INVITE with auth (retry {}) to {}", retry_count, server_addr);
                        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
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

        self.dispatcher.unsubscribe(&cid);
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

#[cfg(test)]
mod ua_tests {
    use super::*;
    use crate::media::sdp::SdpSession;

    #[test]
    fn test_sdp_rtp_address_parsing() {
        let sdp_body = "v=0\r\no=alice 12345 67890 IN IP4 1.2.3.4\r\ns=Session\r\nc=IN IP4 1.2.3.4\r\nt=0 0\r\nm=audio 5000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let sdp = SdpSession::parse(sdp_body).unwrap();
        let media = sdp.media_descriptions.iter().find(|m| m.media_type == "audio").unwrap();
        let ip = sdp.connection_info.split_whitespace().last().unwrap();
        let addr: SocketAddr = format!("{}:{}", ip, media.port).parse().unwrap();
        assert_eq!(addr.to_string(), "1.2.3.4:5000");
    }
}
