use crate::core::{Account, CallState};
use crate::sip::{SipRequest, Method, SipHeaderAccess, SipMessage};
use crate::sip::transport::SipUdpTransport;
use crate::sip::auth::calculate_digest_response;
use anyhow::{Result, anyhow};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

pub struct Call {
    pub id: String,
    pub state: CallState,
    pub remote_uri: String,
}

pub struct UserAgent {
    pub account: Account,
    pub transport: Arc<SipUdpTransport>,
    pub call_id: String,
    pub cseq: u32,
    pub active_call: Option<Call>,
}

impl UserAgent {
    pub fn new(account: Account, transport: Arc<SipUdpTransport>) -> Self {
        Self {
            account,
            transport,
            call_id: Uuid::new_v4().to_string(),
            cseq: 1,
            active_call: None,
        }
    }

    pub async fn register(&mut self, server_addr: SocketAddr) -> Result<()> {
        let uri = format!("sip:{}", self.account.domain);
        let req = SipRequest::new(Method::Register, &uri)
            .with_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-register")
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
            .with_header("To", &format!("<sip:{}@{}>", self.account.username, self.account.domain))
            .with_header("Call-ID", &self.call_id)
            .with_header("CSeq", &format!("{} REGISTER", self.cseq))
            .with_header("Contact", &format!("<sip:{}@127.0.0.1:5060>", self.account.username))
            .with_header("Max-Forwards", "70")
            .with_header("Content-Length", "0");

        let data = req.to_string();
        self.transport.send_to(data.as_bytes(), server_addr).await?;

        // Simplified registration flow for skeleton
        let mut buf = [0u8; 4096];
        let (len, _addr) = self.transport.recv_from(&mut buf).await?;
        let resp_str = String::from_utf8_lossy(&buf[..len]);

        if let Some(SipMessage::Response(res)) = SipMessage::parse(&resp_str) {
            if res.status_code == 401 {
                // Handle challenge
                let authenticate = res.get_header("WWW-Authenticate").ok_or_else(|| anyhow!("Missing WWW-Authenticate"))?;
                // Parse realm and nonce (simplified)
                let realm = authenticate.split("realm=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");
                let nonce = authenticate.split("nonce=\"").nth(1).and_then(|s| s.split('\"').next()).unwrap_or("");

                let password = self.account.password.as_ref().ok_or_else(|| anyhow!("Password required"))?;
                let response = calculate_digest_response(&self.account.username, password, realm, nonce, "REGISTER", &uri);

                self.cseq += 1;
                let auth_header = format!("Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\"",
                    self.account.username, realm, nonce, uri, response);

                let auth_req = req.with_header("CSeq", &format!("{} REGISTER", self.cseq))
                    .with_header("Authorization", &auth_header);

                self.transport.send_to(auth_req.to_string().as_bytes(), server_addr).await?;
            }
        }

        Ok(())
    }

    pub async fn invite(&mut self, remote_uri: &str, server_addr: SocketAddr) -> Result<()> {
        let call_id = Uuid::new_v4().to_string();
        self.cseq += 1;

        let req = SipRequest::new(Method::Invite, remote_uri)
            .with_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-invite")
            .with_header("From", &format!("<sip:{}@{}>;tag={}", self.account.username, self.account.domain, Uuid::new_v4()))
            .with_header("To", remote_uri)
            .with_header("Call-ID", &call_id)
            .with_header("CSeq", &format!("{} INVITE", self.cseq))
            .with_header("Contact", &format!("<sip:{}@127.0.0.1:5060>", self.account.username))
            .with_header("Content-Type", "application/sdp")
            .with_header("Max-Forwards", "70");

        // For skeleton, we'll just transition to Calling state
        self.active_call = Some(Call {
            id: call_id,
            state: CallState::Calling,
            remote_uri: remote_uri.to_string(),
        });

        self.transport.send_to(req.to_string().as_bytes(), server_addr).await?;
        Ok(())
    }
}
