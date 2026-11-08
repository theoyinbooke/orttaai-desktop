//! The HTTPS client for Orttaai Link (`docs/orttaai-link.md` §2). It trusts
//! exactly one key: the Mac's, by the fingerprint learnt while linking. There
//! is no certificate authority and no host-name check, so the same Mac is
//! reached by Wi-Fi address, `.local` name or Tailscale name alike.

use super::model::hex;
use super::wire::ErrorBody;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// No answer at this address (off, asleep, another network).
    #[error("couldn't reach the Mac at {0}: {1}")]
    Unreachable(String, String),
    /// Something answered with a different key: not this Mac.
    #[error("{0} answered, but it isn't the Mac this PC is linked to")]
    WrongKey(String),
    /// The Mac removed this PC.
    #[error("this PC was removed on the Mac")]
    Unlinked,
    /// The Mac needs an Orttaai update to read what this PC sends.
    #[error("update Orttaai on the Mac to keep syncing")]
    UpdateMac,
    #[error("{message}")]
    Refused {
        status: u16,
        code: String,
        message: String,
    },
    #[error("unexpected answer from the Mac: {0}")]
    Protocol(String),
}

impl ClientError {
    /// Worth trying the next address for.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable(..))
    }
}

/// Accepts a server whose public key has this fingerprint.
#[derive(Debug)]
struct PinnedKey {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let key = ec_public_key(end_entity.as_ref()).ok_or(rustls::Error::InvalidCertificate(
            CertificateError::BadEncoding,
        ))?;
        if key_fingerprint(key) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Lowercase hex SHA-256 of a key's X9.63 bytes (`04 || X || Y`).
pub fn key_fingerprint(x963: &[u8]) -> String {
    hex(&Sha256::digest(x963))
}

/// The subject public key (the EC point) of a DER certificate.
pub fn ec_public_key(der: &[u8]) -> Option<&[u8]> {
    let (_, cert, _) = der_next(der)?; // Certificate
    let (_, tbs, _) = der_next(cert)?; // TBSCertificate
    let mut rest = tbs;
    let (tag, _, after) = der_next(rest)?;
    if tag == 0xA0 {
        rest = after; // [0] version
    }
    // serial, signature, issuer, validity, subject
    for _ in 0..5 {
        rest = der_next(rest)?.2;
    }
    let (_, spki, _) = der_next(rest)?; // SubjectPublicKeyInfo
    let (_, _, after_algorithm) = der_next(spki)?;
    let (tag, bits, _) = der_next(after_algorithm)?;
    if tag != 0x03 || bits.first() != Some(&0) {
        return None;
    }
    Some(&bits[1..])
}

/// One DER element: its tag, its content and what follows.
fn der_next(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *input.first()?;
    let first = *input.get(1)?;
    let (length, header) = if first & 0x80 == 0 {
        (first as usize, 2)
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 {
            return None;
        }
        let mut length = 0usize;
        for i in 0..count {
            length = (length << 8) | *input.get(2 + i)? as usize;
        }
        (length, 2 + count)
    };
    let end = header.checked_add(length)?;
    if end > input.len() {
        return None;
    }
    Some((tag, &input[header..end], &input[end..]))
}

/// Talks to one Mac.
#[derive(Clone)]
pub struct LinkClient {
    config: Arc<rustls::ClientConfig>,
    token: Option<String>,
    user_agent: String,
}

impl LinkClient {
    pub fn new(fingerprint: &str, token: Option<String>, app_version: &str) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Arc::new(PinnedKey {
            fingerprint: fingerprint.to_lowercase(),
            provider: provider.clone(),
        });
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("TLS 1.2 and 1.3 with ring")
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        Self {
            config: Arc::new(config),
            token,
            user_agent: format!("Orttaai/{app_version}"),
        }
    }

    pub fn get<R: DeserializeOwned>(
        &self,
        host: &str,
        port: u16,
        path: &str,
        timeout: Duration,
    ) -> Result<R, ClientError> {
        let body = self.request(host, port, "GET", path, None, timeout)?;
        serde_json::from_slice(&body).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        host: &str,
        port: u16,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> Result<R, ClientError> {
        let json = serde_json::to_vec(body).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let answer = self.request(host, port, "POST", path, Some(&json), timeout)?;
        serde_json::from_slice(&answer).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// One request on its own connection; the body of a 2xx answer.
    fn request(
        &self,
        host: &str,
        port: u16,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Vec<u8>, ClientError> {
        let place = format!("{host}:{port}");
        let unreachable =
            |e: &dyn std::fmt::Display| ClientError::Unreachable(place.clone(), e.to_string());
        let address = (host, port)
            .to_socket_addrs()
            .map_err(|e| unreachable(&e))?
            .next()
            .ok_or_else(|| unreachable(&"no address"))?;
        let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(4))
            .map_err(|e| unreachable(&e))?;
        tcp.set_read_timeout(Some(timeout)).ok();
        tcp.set_write_timeout(Some(Duration::from_secs(15))).ok();
        tcp.set_nodelay(true).ok();

        let name = ServerName::try_from("orttaai-link.local").expect("valid name");
        let connection = rustls::ClientConnection::new(self.config.clone(), name)
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let mut stream = rustls::StreamOwned::new(connection, tcp);

        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            place.clone()
        };
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.user_agent
        );
        if let Some(token) = &self.token {
            head.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        if let Some(body) = body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        head.push_str("\r\n");
        let write = |stream: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>| -> std::io::Result<()> {
            stream.write_all(head.as_bytes())?;
            if let Some(body) = body {
                stream.write_all(body)?;
            }
            stream.flush()
        };
        if let Err(e) = write(&mut stream) {
            return Err(classify_tls(&e, &place));
        }

        let (status, answer) = read_response(&mut stream).map_err(|e| classify_tls(&e, &place))?;
        if (200..300).contains(&status) {
            return Ok(answer);
        }
        let error = serde_json::from_slice::<ErrorBody>(&answer).ok();
        let code = error
            .as_ref()
            .map(|e| e.error.code.clone())
            .unwrap_or_default();
        match (status, code.as_str()) {
            (401, "unlinked") | (401, "needs_key") => Err(ClientError::Unlinked),
            (426, _) => Err(ClientError::UpdateMac),
            _ => Err(ClientError::Refused {
                status,
                message: error
                    .map(|e| e.error.message)
                    .unwrap_or_else(|| format!("the Mac answered {status}")),
                code,
            }),
        }
    }
}

fn classify_tls(e: &std::io::Error, place: &str) -> ClientError {
    let text = e.to_string();
    if text.contains("ApplicationVerificationFailure") || text.contains("invalid peer certificate")
    {
        ClientError::WrongKey(place.to_string())
    } else {
        ClientError::Unreachable(place.to_string(), text)
    }
}

/// Reads one HTTP/1.1 response: status and body (Content-Length or chunked).
fn read_response(stream: &mut impl Read) -> std::io::Result<(u16, Vec<u8>)> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 16 * 1024];
    let header_end = loop {
        if let Some(i) = find(&buffer, b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "closed before the answer",
            ));
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.len() > 64 * 1024 {
            return Err(std::io::Error::other("answer headers too long"));
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| std::io::Error::other("no status"))?;
    let header = |name: &str| {
        head.lines().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    let mut body = buffer[header_end..].to_vec();
    if let Some(length) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        while body.len() < length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "answer cut short",
                ));
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(length);
        return Ok((status, body));
    }
    // Chunked or until close: read everything, tolerating a close without
    // TLS close_notify.
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
    }
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        body = dechunk(&body).ok_or_else(|| std::io::Error::other("bad chunked answer"))?;
    }
    Ok((status, body))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = find(data, b"\r\n")?;
        let size_text = std::str::from_utf8(&data[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_sized_answer() {
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let (status, body) = read_response(&mut &raw[..]).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
    }

    #[test]
    fn reads_a_chunked_answer() {
        let raw = b"HTTP/1.1 401 Unauthorized\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        let (status, body) = read_response(&mut &raw[..]).unwrap();
        assert_eq!(status, 401);
        assert_eq!(body, b"{\"a\":1}");
    }

    #[test]
    fn der_lengths() {
        assert_eq!(
            der_next(&[0x30, 0x02, 1, 2, 9]),
            Some((0x30, &[1u8, 2][..], &[9u8][..]))
        );
        assert_eq!(
            der_next(&[0x04, 0x81, 0x01, 7]),
            Some((0x04, &[7u8][..], &[][..]))
        );
        assert_eq!(der_next(&[0x30, 0x05, 1]), None);
    }
}
