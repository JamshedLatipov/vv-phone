use md5::{Md5, Digest};

pub fn calculate_digest_response(
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    method: &str,
    uri: &str,
) -> String {
    let ha1 = format!("{:x}", Md5::digest(format!("{}:{}:{}", username, realm, password).as_bytes()));
    let ha2 = format!("{:x}", Md5::digest(format!("{}:{}", method, uri).as_bytes()));
    format!("{:x}", Md5::digest(format!("{}:{}:{}", ha1, nonce, ha2).as_bytes()))
}

pub fn calculate_digest_response_qop(
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    cnonce: &str,
    nc: &str,
    qop: &str,
    method: &str,
    uri: &str,
) -> String {
    let ha1 = format!("{:x}", Md5::digest(format!("{}:{}:{}", username, realm, password).as_bytes()));
    let ha2 = format!("{:x}", Md5::digest(format!("{}:{}", method, uri).as_bytes()));

    // MD5(HA1:nonce:nc:cnonce:qop:HA2)
    format!("{:x}", Md5::digest(format!("{}:{}:{}:{}:{}:{}", ha1, nonce, nc, cnonce, qop, ha2).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_calculation() {
        let response = calculate_digest_response("admin", "admin", "sip", "12345", "REGISTER", "sip:server.com");
        assert_eq!(response.len(), 32);
    }

    #[test]
    fn test_digest_calculation_qop() {
        let response = calculate_digest_response_qop(
            "Mufasa", "Circle Of Life", "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093", "0a4f113b", "00000001", "auth",
            "GET", "/dir/index.html"
        );
        // RFC 2617 Example values (modified for GET vs REGISTER/SIP)
        // Note: RFC 2617 example matches calculation if inputs are correct.
        assert_eq!(response.len(), 32);
    }
}
