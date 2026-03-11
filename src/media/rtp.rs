use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtpHeader {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub csrc_count: u8,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl RtpHeader {
    pub fn new(payload_type: u8, seq: u16, ts: u32, ssrc: u32) -> Self {
        Self {
            version: 2,
            padding: false,
            extension: false,
            csrc_count: 0,
            marker: false,
            payload_type,
            sequence_number: seq,
            timestamp: ts,
            ssrc,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        let first_byte = (self.version << 6) | ((self.padding as u8) << 5) | ((self.extension as u8) << 4) | (self.csrc_count & 0x0F);
        let second_byte = ((self.marker as u8) << 7) | (self.payload_type & 0x7F);
        bytes.push(first_byte);
        bytes.push(second_byte);
        bytes.extend_from_slice(&self.sequence_number.to_be_bytes());
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.ssrc.to_be_bytes());
        bytes
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 {
            return None;
        }
        let first_byte = bytes[0];
        let second_byte = bytes[1];
        let seq = u16::from_be_bytes([bytes[2], bytes[3]]);
        let ts = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let ssrc = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

        Some(Self {
            version: (first_byte >> 6) & 0x03,
            padding: (first_byte >> 5) & 0x01 == 1,
            extension: (first_byte >> 4) & 0x01 == 1,
            csrc_count: first_byte & 0x0F,
            marker: (second_byte >> 7) & 0x01 == 1,
            payload_type: second_byte & 0x7F,
            sequence_number: seq,
            timestamp: ts,
            ssrc,
        })
    }
}

pub struct RtpPacket {
    pub header: RtpHeader,
    pub payload: Vec<u8>,
}

impl RtpPacket {
    pub fn new(header: RtpHeader, payload: Vec<u8>) -> Self {
        Self { header, payload }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.header.to_bytes();
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let header = RtpHeader::parse(bytes)?;
        let payload = bytes[12..].to_vec();
        Some(Self { header, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_serialization() {
        let header = RtpHeader::new(0, 100, 1000, 12345);
        let payload = vec![0x01, 0x02, 0x03, 0x04];
        let packet = RtpPacket::new(header, payload);

        let bytes = packet.to_bytes();
        assert_eq!(bytes.len(), 16);
        assert_eq!(bytes[0], 0x80); // Version 2
        assert_eq!(bytes[1], 0x00); // Payload type 0

        let parsed = RtpPacket::parse(&bytes).unwrap();
        assert_eq!(parsed.header.sequence_number, 100);
        assert_eq!(parsed.header.ssrc, 12345);
        assert_eq!(parsed.payload, vec![0x01, 0x02, 0x03, 0x04]);
    }
}
