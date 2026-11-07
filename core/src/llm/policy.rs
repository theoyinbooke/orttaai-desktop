//! Local-only endpoint policy (Mac `LocalModelPolicy`): AI features may only
//! talk to loopback, private-LAN, `.local`, Tailscale (100.64/10) and IPv6
//! link/unique-local hosts, and cloud-proxied models (`*-cloud`) are refused.

/// True when `endpoint` (e.g. `http://localhost:11434`) points at a local host.
pub fn is_local_endpoint(endpoint: &str) -> bool {
    let rest = endpoint
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };
    let host = host.to_ascii_lowercase();
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
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        let seg = ip.segments()[0];
        return ip.is_loopback() || (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80;
    }
    false
}

/// Cloud-proxied models (Ollama `*-cloud` / `:cloud`) never run locally.
pub fn is_cloud_model(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("-cloud") || n.ends_with(":cloud") || n.contains("-cloud:")
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
            "http://100.101.1.2:11434",
            "http://box.local:11434",
            "http://[::1]:11434",
            "http://[fd00::1]:1234",
        ] {
            assert!(is_local_endpoint(ok), "{ok}");
        }
        for bad in [
            "https://api.openai.com",
            "http://8.8.8.8:11434",
            "",
            "http://example.com",
        ] {
            assert!(!is_local_endpoint(bad), "{bad}");
        }
    }

    #[test]
    fn cloud_models() {
        assert!(is_cloud_model("gpt-oss:120b-cloud"));
        assert!(!is_cloud_model("llama3.2"));
    }
}
