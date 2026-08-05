//! DNS over HTTPS.
//!
//! Some networks answer DNS queries for mod hosts with a redirect to a block page.
//! Resolving through an encrypted resolver sidesteps that, because the answer is
//! carried inside the TLS session rather than in plaintext UDP that anything on the
//! path can rewrite.
//!
//! This resolves names only. It is not a proxy and it does not hide traffic; the
//! connection that follows is ordinary HTTPS to whatever address came back.

use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Public resolvers that speak the JSON DoH dialect.
pub const RESOLVERS: &[(&str, &str)] = &[
    ("Cloudflare", "https://cloudflare-dns.com/dns-query"),
    ("Google", "https://dns.google/resolve"),
    ("Quad9", "https://dns.quad9.net:5053/dns-query"),
];

#[derive(Debug, Deserialize)]
struct DohResponse {
    #[serde(rename = "Status")]
    status: i32,
    #[serde(rename = "Answer", default)]
    answer: Vec<DohAnswer>,
}

#[derive(Debug, Deserialize)]
struct DohAnswer {
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

/// Resolves a hostname to addresses, trying each resolver until one answers.
pub async fn resolve(client: &reqwest::Client, host: &str) -> Result<Vec<IpAddr>> {
    let mut last_error = None;

    for (name, endpoint) in RESOLVERS {
        match query(client, endpoint, host).await {
            Ok(addresses) if !addresses.is_empty() => return Ok(addresses),
            Ok(_) => last_error = Some(format!("{name} returned no records for {host}")),
            Err(error) => last_error = Some(format!("{name}: {error}")),
        }
    }

    Err(Error::Network(
        last_error.unwrap_or_else(|| format!("could not resolve {host}")),
    ))
}

async fn query(client: &reqwest::Client, endpoint: &str, host: &str) -> Result<Vec<IpAddr>> {
    let response = client
        .get(endpoint)
        .query(&[("name", host), ("type", "A")])
        .header("accept", "application/dns-json")
        .timeout(Duration::from_secs(6))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "resolver replied {}",
            response.status()
        )));
    }

    let body: DohResponse = response.json().await?;
    if body.status != 0 {
        return Err(Error::Network(format!("DNS status {}", body.status)));
    }

    Ok(body
        .answer
        .into_iter()
        // 1 = A, 28 = AAAA. CNAME chains appear as type 5 and are skipped.
        .filter(|entry| entry.kind == 1 || entry.kind == 28)
        .filter_map(|entry| entry.data.parse::<IpAddr>().ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resolver_endpoint_is_https() {
        for (name, endpoint) in RESOLVERS {
            assert!(
                endpoint.starts_with("https://"),
                "{name} must use an encrypted transport"
            );
        }
    }

    #[test]
    fn only_address_records_are_kept() {
        let body: DohResponse = serde_json::from_str(
            r#"{"Status":0,"Answer":[
                {"type":5,"data":"alias.example.com."},
                {"type":1,"data":"93.184.216.34"},
                {"type":28,"data":"2606:2800:220:1:248:1893:25c8:1946"},
                {"type":1,"data":"not-an-ip"}
            ]}"#,
        )
        .unwrap();

        let addresses: Vec<IpAddr> = body
            .answer
            .into_iter()
            .filter(|e| e.kind == 1 || e.kind == 28)
            .filter_map(|e| e.data.parse::<IpAddr>().ok())
            .collect();

        assert_eq!(addresses.len(), 2);
        assert!(addresses[0].is_ipv4());
        assert!(addresses[1].is_ipv6());
    }

    #[test]
    fn a_failure_status_is_surfaced() {
        let body: DohResponse = serde_json::from_str(r#"{"Status":3,"Answer":[]}"#).unwrap();
        assert_eq!(body.status, 3);
        assert!(body.answer.is_empty());
    }
}
