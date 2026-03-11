use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdpSession {
    pub version: u32,
    pub owner: String,
    pub session_name: String,
    pub connection_info: String,
    pub time_description: String,
    pub media_descriptions: Vec<SdpMediaDescription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdpMediaDescription {
    pub media_type: String,
    pub port: u16,
    pub transport: String,
    pub formats: Vec<String>,
    pub attributes: Vec<String>,
}

impl SdpSession {
    pub fn new(owner_name: &str, session_name: &str, connection_ip: &str) -> Self {
        Self {
            version: 0,
            owner: format!("{} 12345 67890 IN IP4 {}", owner_name, connection_ip),
            session_name: session_name.to_string(),
            connection_info: format!("IN IP4 {}", connection_ip),
            time_description: "0 0".to_string(),
            media_descriptions: Vec::new(),
        }
    }

    pub fn add_media(&mut self, media: SdpMediaDescription) {
        self.media_descriptions.push(media);
    }

    pub fn to_string(&self) -> String {
        let mut s = format!("v={}\r\n", self.version);
        s.push_str(&format!("o={}\r\n", self.owner));
        s.push_str(&format!("s={}\r\n", self.session_name));
        s.push_str(&format!("c={}\r\n", self.connection_info));
        s.push_str(&format!("t={}\r\n", self.time_description));
        for media in &self.media_descriptions {
            s.push_str(&format!("m={} {} {} {}\r\n", media.media_type, media.port, media.transport, media.formats.join(" ")));
            for attr in &media.attributes {
                s.push_str(&format!("a={}\r\n", attr));
            }
        }
        s
    }

    pub fn parse(input: &str) -> Option<Self> {
        let mut session = SdpSession::new("", "", "");
        let mut current_media: Option<SdpMediaDescription> = None;

        for line in input.lines() {
            let (k, v) = line.split_once('=')?;
            match k {
                "v" => session.version = v.parse().ok()?,
                "o" => session.owner = v.to_string(),
                "s" => session.session_name = v.to_string(),
                "c" => session.connection_info = v.to_string(),
                "t" => session.time_description = v.to_string(),
                "m" => {
                    if let Some(m) = current_media.take() {
                        session.media_descriptions.push(m);
                    }
                    let parts: Vec<&str> = v.split_whitespace().collect();
                    if parts.len() >= 4 {
                        current_media = Some(SdpMediaDescription {
                            media_type: parts[0].to_string(),
                            port: parts[1].parse().ok()?,
                            transport: parts[2].to_string(),
                            formats: parts[3..].iter().map(|s| s.to_string()).collect(),
                            attributes: Vec::new(),
                        });
                    }
                }
                "a" => {
                    if let Some(ref mut m) = current_media {
                        m.attributes.push(v.to_string());
                    }
                }
                _ => {}
            }
        }

        if let Some(m) = current_media {
            session.media_descriptions.push(m);
        }

        Some(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdp_serialization() {
        let mut session = SdpSession::new("alice", "Session A", "127.0.0.1");
        session.add_media(SdpMediaDescription {
            media_type: "audio".to_string(),
            port: 4000,
            transport: "RTP/AVP".to_string(),
            formats: vec!["0".to_string(), "8".to_string()],
            attributes: vec!["rtpmap:0 PCMU/8000".to_string()],
        });

        let s = session.to_string();
        assert!(s.contains("v=0"));
        assert!(s.contains("s=Session A"));
        assert!(s.contains("m=audio 4000 RTP/AVP 0 8"));
        assert!(s.contains("a=rtpmap:0 PCMU/8000"));
    }

    #[test]
    fn test_sdp_parsing() {
        let raw = "v=0\r\no=alice 12345 67890 IN IP4 127.0.0.1\r\ns=Session A\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0 8\r\na=rtpmap:0 PCMU/8000\r\n";
        let session = SdpSession::parse(raw).unwrap();
        assert_eq!(session.version, 0);
        assert_eq!(session.session_name, "Session A");
        assert_eq!(session.media_descriptions.len(), 1);
        assert_eq!(session.media_descriptions[0].port, 4000);
        assert_eq!(session.media_descriptions[0].attributes[0], "rtpmap:0 PCMU/8000");
    }
}
