pub mod transport;

use std::fmt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Invite,
    Ack,
    Bye,
    Cancel,
    Options,
    Register,
    Notify,
    Subscribe,
    Message,
    Refer,
    Update,
    Info,
    Prack,
    Publish,
    Unknown(String),
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Method::Invite => "INVITE",
            Method::Ack => "ACK",
            Method::Bye => "BYE",
            Method::Cancel => "CANCEL",
            Method::Options => "OPTIONS",
            Method::Register => "REGISTER",
            Method::Notify => "NOTIFY",
            Method::Subscribe => "SUBSCRIBE",
            Method::Message => "MESSAGE",
            Method::Refer => "REFER",
            Method::Update => "UPDATE",
            Method::Info => "INFO",
            Method::Prack => "PRACK",
            Method::Publish => "PUBLISH",
            Method::Unknown(s) => s,
        };
        write!(f, "{}", s)
    }
}

impl From<&str> for Method {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "INVITE" => Method::Invite,
            "ACK" => Method::Ack,
            "BYE" => Method::Bye,
            "CANCEL" => Method::Cancel,
            "OPTIONS" => Method::Options,
            "REGISTER" => Method::Register,
            "NOTIFY" => Method::Notify,
            "SUBSCRIBE" => Method::Subscribe,
            "MESSAGE" => Method::Message,
            "REFER" => Method::Refer,
            "UPDATE" => Method::Update,
            "INFO" => Method::Info,
            "PRACK" => Method::Prack,
            "PUBLISH" => Method::Publish,
            _ => Method::Unknown(s.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SipRequest {
    pub method: Method,
    pub uri: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SipResponse {
    pub status_code: u16,
    pub reason: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl SipRequest {
    pub fn new(method: Method, uri: &str) -> Self {
        Self {
            method,
            uri: uri.to_string(),
            version: "SIP/2.0".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn to_string(&self) -> String {
        let mut s = format!("{} {} {}\r\n", self.method, self.uri, self.version);
        for (k, v) in &self.headers {
            s.push_str(&format!("{}: {}\r\n", k, v));
        }
        s.push_str("\r\n");
        let body_str = String::from_utf8_lossy(&self.body).to_string();
        s.push_str(&body_str);
        s
    }

    pub fn parse(input: &str) -> Option<Self> {
        let mut lines = input.split("\r\n");
        let first_line = lines.next()?;
        let mut parts = first_line.split_whitespace();
        let method = Method::from(parts.next()?);
        let uri = parts.next()?.to_string();
        let version = parts.next()?.to_string();

        let mut headers = Vec::new();
        while let Some(line) = lines.next() {
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }

        let body = lines.collect::<Vec<&str>>().join("\r\n").into_bytes();

        Some(SipRequest {
            method,
            uri,
            version,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_request_serialization() {
        let req = SipRequest::new(Method::Invite, "sip:alice@atlanta.com")
            .with_header("From", "sip:bob@biloxi.com");
        let s = req.to_string();
        assert!(s.contains("INVITE sip:alice@atlanta.com SIP/2.0"));
        assert!(s.contains("From: sip:bob@biloxi.com"));
    }

    #[test]
    fn test_sip_request_parsing() {
        let raw = "INVITE sip:alice@atlanta.com SIP/2.0\r\nFrom: sip:bob@biloxi.com\r\n\r\nv=0";
        let req = SipRequest::parse(raw).unwrap();
        assert_eq!(req.method, Method::Invite);
        assert_eq!(req.uri, "sip:alice@atlanta.com");
        assert_eq!(req.headers[0].0, "From");
        assert_eq!(req.headers[0].1, "sip:bob@biloxi.com");
        assert_eq!(req.body, b"v=0");
    }
}
