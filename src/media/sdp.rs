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
            if line.is_empty() { continue; }
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

    pub fn negotiate(&self, other: &SdpSession) -> Option<SdpSession> {
        let mut result = SdpSession::new("softphone", "Negotiated Session", "0.0.0.0");

        for my_media in &self.media_descriptions {
            if let Some(other_media) = other.media_descriptions.iter().find(|m| m.media_type == my_media.media_type) {
                let mut common_formats = Vec::new();
                for fmt in &my_media.formats {
                    if other_media.formats.contains(fmt) {
                        common_formats.push(fmt.clone());
                    }
                }

                if !common_formats.is_empty() {
                    let mut common_attributes = Vec::new();
                    for fmt in &common_formats {
                        let rtpmap = format!("rtpmap:{}", fmt);
                        if let Some(attr) = my_media.attributes.iter().find(|a| a.starts_with(&rtpmap)) {
                            common_attributes.push(attr.clone());
                        }
                        if let Some(attr) = other_media.attributes.iter().find(|a| a.starts_with(&rtpmap)) {
                            if !common_attributes.contains(attr) {
                                common_attributes.push(attr.clone());
                            }
                        }
                    }

                    result.add_media(SdpMediaDescription {
                        media_type: my_media.media_type.clone(),
                        port: my_media.port,
                        transport: my_media.transport.clone(),
                        formats: common_formats,
                        attributes: common_attributes,
                    });
                }
            }
        }

        if result.media_descriptions.is_empty() {
            None
        } else {
            Some(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdp_negotiation() {
        let mut offer = SdpSession::new("alice", "Offer", "127.0.0.1");
        offer.add_media(SdpMediaDescription {
            media_type: "audio".to_string(),
            port: 4000,
            transport: "RTP/AVP".to_string(),
            formats: vec!["0".to_string(), "8".to_string(), "96".to_string()],
            attributes: vec![
                "rtpmap:0 PCMU/8000".to_string(),
                "rtpmap:8 PCMA/8000".to_string(),
                "rtpmap:96 OPUS/48000/2".to_string(),
            ],
        });

        let mut answer = SdpSession::new("bob", "Answer", "127.0.0.2");
        answer.add_media(SdpMediaDescription {
            media_type: "audio".to_string(),
            port: 5000,
            transport: "RTP/AVP".to_string(),
            formats: vec!["96".to_string(), "0".to_string()],
            attributes: vec![
                "rtpmap:96 OPUS/48000/2".to_string(),
                "rtpmap:0 PCMU/8000".to_string(),
            ],
        });

        let negotiated = offer.negotiate(&answer).unwrap();
        assert_eq!(negotiated.media_descriptions[0].formats.len(), 2);
        assert!(negotiated.media_descriptions[0].formats.contains(&"0".to_string()));
        assert!(negotiated.media_descriptions[0].formats.contains(&"96".to_string()));
    }
}
