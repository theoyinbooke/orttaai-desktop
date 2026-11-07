//! Local-only endpoint policy (Mac `LocalModelPolicy`): AI features may only
//! talk to loopback, private-LAN, `.local`, Tailscale (100.64/10, `*.ts.net`),
//! IPv6 link/unique-local hosts and bare machine names, and cloud-proxied
//! models (`*-cloud`) are refused.

/// Shown when a request or endpoint is refused.
pub const REFUSAL_MESSAGE: &str =
    "Orttaai only uses models on your own hardware. Cloud models aren't supported.";

/// True when `endpoint` (e.g. `http://localhost:11434`) points at a local host.
pub fn is_local_endpoint(endpoint: &str) -> bool {
    let trimmed = endpoint.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("http://") {
        r
    } else if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if lower.contains("://") {
        // Only http(s) servers are supported.
        return false;
    } else {
        lower.as_str()
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Drop any `user:pass@`.
    let authority = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };
    if host.is_empty() {
        return false;
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    if host.ends_with(".ts.net") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        let o = ip.octets();
        return ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || (o[0] == 100 && (64..128).contains(&o[1]));
    }
    if host.contains(':') {
        let address = host.split('%').next().unwrap_or(host);
        return match address.parse::<std::net::Ipv6Addr>() {
            Ok(ip) => {
                let seg = ip.segments()[0];
                ip.is_loopback() || (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
            }
            Err(_) => false,
        };
    }
    // A dotted name that isn't an IPv4 address is a public DNS name; a bare
    // machine name ("mac-studio", "gpu-box") resolves only on the local network.
    !host.contains('.') && !host.chars().all(|c| c.is_ascii_digit())
}

/// Cloud-proxied models (Ollama `gpt-oss:120b-cloud`, `kimi-k2:cloud`) never
/// run locally.
pub fn is_cloud_model(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    match lowered.split_once(':') {
        Some((_, tag)) => tag == "cloud" || tag.ends_with("-cloud"),
        None => lowered.ends_with("-cloud"),
    }
}

/// Refuse a cloud model before any request is built.
pub fn require_local_model(model: &str, provider: &str) -> crate::Result<()> {
    if is_cloud_model(model) {
        return Err(crate::CoreError::Llm(format!(
            "{REFUSAL_MESSAGE} \u{201C}{}\u{201D} runs in {provider}'s cloud.",
            model.trim()
        )));
    }
    Ok(())
}

/// Refuse an endpoint outside the user's own hardware.
pub fn require_local_endpoint(endpoint: &str, provider: &str) -> crate::Result<()> {
    if !is_local_endpoint(endpoint) {
        return Err(crate::CoreError::Llm(format!(
            "{REFUSAL_MESSAGE} The {provider} address must be on this computer or your local network."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_endpoints() {
        for ok in [
            "http://localhost:11434",
            "http://127.0.0.1:1234/v1",
            "http://192.168.1.20:11434",
            "http://10.0.0.5",
            "http://172.20.0.2:11434",
            "http://169.254.3.4",
            "http://100.101.1.2:11434",
            "http://box.local:11434",
            "http://gpu.tail1234.ts.net:11434",
            "http://mac-studio:11434",
            "https://LOCALHOST:1234",
            "http://[::1]:11434",
            "http://[fd00::1]:1234",
            "http://[fe80::1%25en0]:1234",
        ] {
            assert!(is_local_endpoint(ok), "{ok}");
        }
        for bad in [
            "https://api.openai.com",
            "http://8.8.8.8:11434",
            "http://172.32.0.1",
            "http://100.200.0.1",
            "",
            "http://",
            "http://example.com",
            "ftp://localhost",
            "http://[2001:4860::1]:80",
            "http://evil.com@8.8.8.8",
        ] {
            assert!(!is_local_endpoint(bad), "{bad}");
        }
    }

    #[test]
    fn cloud_models() {
        assert!(is_cloud_model("gpt-oss:120b-cloud"));
        assert!(is_cloud_model("kimi-k2:cloud"));
        assert!(is_cloud_model("deepseek-v3.1:671b-cloud"));
        assert!(is_cloud_model("something-cloud"));
        assert!(!is_cloud_model("llama3.2"));
        assert!(!is_cloud_model("cloudy:latest"));
        assert!(require_local_model("gpt-oss:120b-cloud", "Ollama")
            .unwrap_err()
            .to_string()
            .contains("own hardware"));
        assert!(require_local_model("gemma4:e2b", "Ollama").is_ok());
        assert!(require_local_endpoint("https://api.openai.com", "Ollama").is_err());
    }
}
