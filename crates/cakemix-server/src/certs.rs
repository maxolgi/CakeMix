//! Web (HTTPS) certificate lifecycle.
//!
//! Distinct from the WebTransport identity (built via `websrt::cert`, ≤14-day
//! validity because `serverCertificateHashes` pinning caps it — see
//! vendor/WebSRT/docs/embedding.md "Cert modes"). The web cert only needs to
//! satisfy the browser's HTTPS requirement for the page + AudioWorklet, so it
//! is a self-signed PEM persisted at the repo root (cert.pem/key.pem, already
//! gitignored) with ~13 months of validity (Chrome caps public certs at 398
//! days). Regenerated when missing, unreadable, or close to expiry — SANs
//! cover localhost plus all local LAN IPs so other machines on the LAN get a
//! secure context too.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use anyhow::{Context, Result};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, DnType, KeyPair, SanType};
use rustls_pki_types::pem::PemObject;
use time::{Duration, OffsetDateTime};

/// ~13 months; Chrome rejects public certs with >398-day validity.
const VALIDITY_DAYS: i64 = 397;
/// Regenerate the persisted cert when less than this remains.
const EXPIRY_SLACK_DAYS: i64 = 30;

/// A ready-to-use web cert as PEM bytes, plus metadata for logging.
pub struct WebCert {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub not_after: OffsetDateTime,
    pub sans: Vec<String>,
}

/// Load the persisted web cert, generating a fresh one when it is missing,
/// unreadable, or within `EXPIRY_SLACK_DAYS` of expiry.
pub fn ensure_web_cert(cert_path: &Path, key_path: &Path) -> Result<WebCert> {
    if let Some(existing) = load_persisted(cert_path, key_path) {
        println!(
            "reusing persisted web cert {} (expires {}, SANs: [{}])",
            cert_path.display(),
            existing.not_after.date(),
            existing.sans.join(", ")
        );
        return Ok(existing);
    }
    generate(cert_path, key_path)
}

/// Read + validate the persisted pair. Returns `None` for anything that
/// warrants regeneration.
fn load_persisted(cert_path: &Path, key_path: &Path) -> Option<WebCert> {
    if !cert_path.exists() || !key_path.exists() {
        return None;
    }
    // Unreadable files (or a key rustls will reject) count as "unreadable".
    let cert_pem = std::fs::read(cert_path).ok()?;
    let key_pem = std::fs::read(key_path).ok()?;
    rustls_pki_types::PrivateKeyDer::from_pem_slice(&key_pem).ok()?;
    let (not_after, sans) = parse_cert(&cert_pem)?;

    if not_after < OffsetDateTime::now_utc() + Duration::days(EXPIRY_SLACK_DAYS) {
        println!("persisted web cert expires soon ({not_after}); regenerating");
        return None;
    }

    Some(WebCert {
        cert_pem,
        key_pem,
        not_after,
        sans,
    })
}

fn generate(cert_path: &Path, key_path: &Path) -> Result<WebCert> {
    let now = OffsetDateTime::now_utc();

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    params.not_before = now - Duration::days(1); // tolerate clock skew
    params.not_after = now + Duration::days(VALIDITY_DAYS);
    params.subject_alt_names = web_cert_sans();

    let sans: Vec<String> = params.subject_alt_names.iter().map(san_to_string).collect();

    let key_pair = KeyPair::generate().context("generate web cert key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("self-sign web cert")?;
    let web_cert = WebCert {
        cert_pem: cert.pem().into_bytes(),
        key_pem: key_pair.serialize_pem().into_bytes(),
        not_after: params.not_after,
        sans,
    };

    std::fs::write(cert_path, &web_cert.cert_pem)
        .with_context(|| format!("write {}", cert_path.display()))?;
    std::fs::write(key_path, &web_cert.key_pem)
        .with_context(|| format!("write {}", key_path.display()))?;
    println!(
        "generated new web cert {} (expires {}, SANs: [{}])",
        cert_path.display(),
        web_cert.not_after.date(),
        web_cert.sans.join(", ")
    );
    Ok(web_cert)
}

/// DNS localhost + loopback IPs + every local LAN IP (AudioWorklet requires a
/// secure context on the machines that reach the server).
fn web_cert_sans() -> Vec<SanType> {
    let mut sans = vec![
        SanType::DnsName(Ia5String::try_from("localhost").unwrap()),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ];
    for ip in local_lan_ips() {
        println!("Adding SAN for local IP: {ip}");
        sans.push(SanType::IpAddress(ip));
    }
    sans
}

fn san_to_string(san: &SanType) -> String {
    match san {
        SanType::DnsName(name) => format!("DNS:{}", name.as_str()),
        SanType::IpAddress(ip) => format!("IP:{ip}"),
        _ => "other".to_string(),
    }
}

/// Extract the notAfter date and SAN list from a PEM certificate.
fn parse_cert(cert_pem: &[u8]) -> Option<(OffsetDateTime, Vec<String>)> {
    use x509_parser::extensions::ParsedExtension;
    use x509_parser::prelude::*;

    let (_, pem) = parse_x509_pem(cert_pem).ok()?;
    let (_, cert) = X509Certificate::from_der(&pem.contents).ok()?;
    let not_after = cert.validity().not_after.to_datetime();

    let sans = cert
        .extensions()
        .iter()
        .find_map(|ext| match ext.parsed_extension() {
            ParsedExtension::SubjectAlternativeName(san) => Some(
                san.general_names
                    .iter()
                    .filter_map(|gn| match gn {
                        GeneralName::DNSName(name) => Some(format!("DNS:{name}")),
                        GeneralName::IPAddress(octets) => match octets.len() {
                            4 => {
                                let bytes: [u8; 4] = octets[..4].try_into().unwrap();
                                Some(format!("IP:{}", Ipv4Addr::from(bytes)))
                            }
                            16 => {
                                let bytes: [u8; 16] = octets[..16].try_into().unwrap();
                                Some(format!("IP:{}", Ipv6Addr::from(bytes)))
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();

    Some((not_after, sans))
}

/// Dependency-free way to find the primary LAN IP: a UDP "connect"
/// sends no packets but makes the OS pick the route's source address.
fn local_lan_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                ips.push(addr.ip());
            }
        }
    }
    ips
}
