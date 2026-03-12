use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
pub enum SipMessage {
    Request(SipRequest),
    Response(SipResponse),
}

pub trait SipHeaderAccess {
    fn get_headers(&self) -> &Vec<(String, String)>;
    fn get_headers_mut(&mut self) -> &mut Vec<(String, String)>;

    fn get_header(&self, name: &str) -> Option<&String> {
        self.get_headers().iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }

    fn get_all_headers(&self, name: &str) -> Vec<&String> {
        self.get_headers().iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
            .collect()
    }

    fn set_header(&mut self, name: &str, value: &str) {
        if let Some(pos) = self.get_headers().iter().position(|(k, _)| k.eq_ignore_ascii_case(name)) {
            self.get_headers_mut()[pos].1 = value.to_string();
        } else {
            self.get_headers_mut().push((name.to_string(), value.to_string()));
        }
    }

    fn add_header(&mut self, name: &str, value: &str) {
        self.get_headers_mut().push((name.to_string(), value.to_string()));
    }

    fn via(&self) -> Option<&String> { self.get_header("Via") }
    fn from(&self) -> Option<&String> { self.get_header("From") }
    fn to(&self) -> Option<&String> { self.get_header("To") }
    fn call_id(&self) -> Option<&String> { self.get_header("Call-ID") }
    fn cseq(&self) -> Option<&String> { self.get_header("CSeq") }
    fn contact(&self) -> Option<&String> { self.get_header("Contact") }
    fn p_access_network_info(&self) -> Option<&String> { self.get_header("P-Access-Network-Info") }
    fn content_length(&self) -> Option<usize> {
        self.get_header("Content-Length").and_then(|v| v.parse().ok())
    }
}

impl SipHeaderAccess for SipRequest {
    fn get_headers(&self) -> &Vec<(String, String)> { &self.headers }
    fn get_headers_mut(&mut self) -> &mut Vec<(String, String)> { &mut self.headers }
}

impl SipHeaderAccess for SipResponse {
    fn get_headers(&self) -> &Vec<(String, String)> { &self.headers }
    fn get_headers_mut(&mut self) -> &mut Vec<(String, String)> { &mut self.headers }
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
        self.set_header(name, value);
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

impl SipResponse {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            version: "SIP/2.0".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.set_header(name, value);
        self
    }

    pub fn to_string(&self) -> String {
        let mut s = format!("{} {} {}\r\n", self.version, self.status_code, self.reason);
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
        let version = parts.next()?.to_string();
        let status_code = parts.next()?.parse().ok()?;
        let reason = parts.collect::<Vec<&str>>().join(" ");

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

        Some(SipResponse {
            status_code,
            reason,
            version,
            headers,
            body,
        })
    }
}

impl SipMessage {
    pub fn parse(input: &str) -> Option<Self> {
        if input.starts_with("SIP/2.0") {
            SipResponse::parse(input).map(SipMessage::Response)
        } else {
            SipRequest::parse(input).map(SipMessage::Request)
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            SipMessage::Request(req) => req.to_string(),
            SipMessage::Response(res) => res.to_string(),
        }
    }
}

pub mod auth;
pub mod transport;
pub mod ua;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_header_accessors() {
        let mut req = SipRequest::new(Method::Invite, "sip:alice@atlanta.com")
            .with_header("Via", "SIP/2.0/UDP 127.0.0.1:5060")
            .with_header("From", "sip:bob@biloxi.com")
            .with_header("To", "sip:alice@atlanta.com")
            .with_header("Call-ID", "12345")
            .with_header("CSeq", "1 INVITE")
            .with_header("Contact", "sip:bob@127.0.0.1")
            .with_header("Content-Length", "0");

        assert_eq!(req.via().unwrap(), "SIP/2.0/UDP 127.0.0.1:5060");
        assert_eq!(req.from().unwrap(), "sip:bob@biloxi.com");
        assert_eq!(req.to().unwrap(), "sip:alice@atlanta.com");
        assert_eq!(req.call_id().unwrap(), "12345");
        assert_eq!(req.cseq().unwrap(), "1 INVITE");
        assert_eq!(req.contact().unwrap(), "sip:bob@127.0.0.1");
        assert_eq!(req.content_length().unwrap(), 0);

        req.set_header("Content-Length", "10");
        assert_eq!(req.content_length().unwrap(), 10);
    }

    #[test]
    fn test_duplicate_headers() {
        let mut req = SipRequest::new(Method::Invite, "sip:alice@atlanta.com");
        req.add_header("Via", "SIP/2.0/UDP 127.0.0.1:5060");
        req.add_header("Via", "SIP/2.0/UDP 192.168.1.1:5060");

        let vias = req.get_all_headers("Via");
        assert_eq!(vias.len(), 2);
        assert_eq!(vias[0], "SIP/2.0/UDP 127.0.0.1:5060");
        assert_eq!(vias[1], "SIP/2.0/UDP 192.168.1.1:5060");
    }

    #[test]
    fn test_p_access_network_info() {
        let req = SipRequest::new(Method::Register, "sip:server.com")
            .with_header("P-Access-Network-Info", "3GPP-UTRAN-TDD; utran-cell-id-3gpp=234151D0FCE11");
        assert_eq!(req.p_access_network_info().unwrap(), "3GPP-UTRAN-TDD; utran-cell-id-3gpp=234151D0FCE11");
    }

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

    #[test]
    fn test_sip_response_serialization() {
        let res = SipResponse::new(200, "OK")
            .with_header("To", "sip:alice@atlanta.com");
        let s = res.to_string();
        assert!(s.contains("SIP/2.0 200 OK"));
        assert!(s.contains("To: sip:alice@atlanta.com"));
    }

    #[test]
    fn test_sip_response_parsing() {
        let raw = "SIP/2.0 200 OK\r\nTo: sip:alice@atlanta.com\r\n\r\nv=0";
        let res = SipResponse::parse(raw).unwrap();
        assert_eq!(res.status_code, 200);
        assert_eq!(res.reason, "OK");
        assert_eq!(res.headers[0].0, "To");
        assert_eq!(res.headers[0].1, "sip:alice@atlanta.com");
        assert_eq!(res.body, b"v=0");
    }

    #[test]
    fn test_sip_message_parsing() {
        let raw_req = "INVITE sip:alice@atlanta.com SIP/2.0\r\n\r\n";
        let raw_res = "SIP/2.0 200 OK\r\n\r\n";

        match SipMessage::parse(raw_req).unwrap() {
            SipMessage::Request(req) => assert_eq!(req.method, Method::Invite),
            _ => panic!("Expected Request"),
        }

        match SipMessage::parse(raw_res).unwrap() {
            SipMessage::Response(res) => assert_eq!(res.status_code, 200),
            _ => panic!("Expected Response"),
        }
    }
}
