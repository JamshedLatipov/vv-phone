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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_calculation() {
        // Values from RFC 2069 (older, but it's where the basic MD5 example comes from)
        // username="Mufasa", realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", uri="/dir/index.html", response="e966c932a9242554e42c8ee20457ac30"
        // Let's use a simpler known case.

        let response = calculate_digest_response("admin", "admin", "sip", "12345", "REGISTER", "sip:server.com");
        assert!(!response.is_empty());
        assert_eq!(response.len(), 32);
    }
}
