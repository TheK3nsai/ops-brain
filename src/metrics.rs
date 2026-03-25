use serde::Serialize;
use std::collections::HashMap;

/// Status of a single Uptime Kuma monitor, parsed from Prometheus metrics
#[derive(Debug, Clone, Serialize)]
pub struct MonitorStatus {
    pub name: String,
    pub monitor_type: String,
    pub url: String,
    pub hostname: String,
    pub port: String,
    /// 1=up, 0=down, 2=pending, 3=maintenance
    pub status: i64,
    pub status_text: String,
    /// Response time in milliseconds (if available)
    pub response_time_ms: Option<f64>,
    /// SSL cert days remaining (if available)
    pub cert_days_remaining: Option<f64>,
    /// Whether SSL cert is valid (if available)
    pub cert_is_valid: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSummary {
    pub total: usize,
    pub up: usize,
    pub down: usize,
    pub pending: usize,
    pub maintenance: usize,
    pub monitors: Vec<MonitorStatus>,
}

/// Configuration for connecting to Uptime Kuma
#[derive(Debug, Clone)]
pub struct UptimeKumaConfig {
    pub base_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

fn status_text(code: i64) -> String {
    match code {
        1 => "up".to_string(),
        0 => "down".to_string(),
        2 => "pending".to_string(),
        3 => "maintenance".to_string(),
        _ => format!("unknown({code})"),
    }
}

/// Fetch and parse /metrics from Uptime Kuma (Prometheus exposition format)
pub async fn fetch_metrics(config: &UptimeKumaConfig) -> Result<MetricsSummary, String> {
    let url = format!("{}/metrics", config.base_url.trim_end_matches('/'));

    let client = reqwest::Client::new();
    let mut request = client.get(&url);

    if let (Some(user), Some(pass)) = (&config.username, &config.password) {
        request = request.basic_auth(user, Some(pass));
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to fetch metrics from {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Uptime Kuma /metrics returned HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read metrics body: {e}"))?;

    parse_prometheus_metrics(&body)
}

/// Parse Prometheus exposition format text into structured monitor data.
///
/// We look for these metric families:
///   monitor_status{...} gauge
///   monitor_response_time{...} gauge
///   monitor_cert_days_remaining{...} gauge
///   monitor_cert_is_valid{...} gauge
fn parse_prometheus_metrics(text: &str) -> Result<MetricsSummary, String> {
    // Collect per-monitor data keyed by monitor_name
    let mut monitors: HashMap<String, MonitorStatus> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse: metric_name{label="val",...} value
        if let Some((metric_with_labels, value_str)) = line.rsplit_once(' ') {
            let value: f64 = match value_str.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Split metric name from labels
            let (metric_name, labels) = if let Some(brace_start) = metric_with_labels.find('{') {
                let name = &metric_with_labels[..brace_start];
                let label_str = &metric_with_labels[brace_start + 1..];
                let label_str = label_str.trim_end_matches('}');
                (name, parse_labels(label_str))
            } else {
                continue; // Skip metrics without labels
            };

            let monitor_name = match labels.get("monitor_name") {
                Some(name) => name.clone(),
                None => continue,
            };

            let entry = monitors.entry(monitor_name.clone()).or_insert_with(|| {
                MonitorStatus {
                    name: monitor_name,
                    monitor_type: labels.get("monitor_type").cloned().unwrap_or_default(),
                    url: labels.get("monitor_url").cloned().unwrap_or_default(),
                    hostname: labels.get("monitor_hostname").cloned().unwrap_or_default(),
                    port: labels.get("monitor_port").cloned().unwrap_or_default(),
                    status: 2, // default pending
                    status_text: "pending".to_string(),
                    response_time_ms: None,
                    cert_days_remaining: None,
                    cert_is_valid: None,
                }
            });

            match metric_name {
                "monitor_status" => {
                    entry.status = value as i64;
                    entry.status_text = status_text(value as i64);
                }
                "monitor_response_time" => {
                    if value >= 0.0 {
                        entry.response_time_ms = Some(value);
                    }
                }
                "monitor_cert_days_remaining" => {
                    entry.cert_days_remaining = Some(value);
                }
                "monitor_cert_is_valid" => {
                    entry.cert_is_valid = Some(value == 1.0);
                }
                _ => {}
            }
        }
    }

    let mut monitor_list: Vec<MonitorStatus> = monitors.into_values().collect();
    monitor_list.sort_by(|a, b| a.name.cmp(&b.name));

    let up = monitor_list.iter().filter(|m| m.status == 1).count();
    let down = monitor_list.iter().filter(|m| m.status == 0).count();
    let pending = monitor_list.iter().filter(|m| m.status == 2).count();
    let maintenance = monitor_list.iter().filter(|m| m.status == 3).count();

    Ok(MetricsSummary {
        total: monitor_list.len(),
        up,
        down,
        pending,
        maintenance,
        monitors: monitor_list,
    })
}

/// Parse Prometheus label string: key1="val1",key2="val2"
fn parse_labels(label_str: &str) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    let mut remaining = label_str;

    while !remaining.is_empty() {
        // Find key=
        let eq_pos = match remaining.find('=') {
            Some(p) => p,
            None => break,
        };
        let key = remaining[..eq_pos].trim().trim_start_matches(',').trim();
        remaining = &remaining[eq_pos + 1..];

        // Value is quoted: "..."
        if remaining.starts_with('"') {
            remaining = &remaining[1..];
            // Find closing quote (handle escaped quotes)
            let mut value = String::new();
            let mut chars = remaining.chars();
            loop {
                match chars.next() {
                    Some('\\') => {
                        if let Some(c) = chars.next() {
                            value.push(c);
                        }
                    }
                    Some('"') => break,
                    Some(c) => value.push(c),
                    None => break,
                }
            }
            remaining = chars.as_str();
            // Skip comma
            remaining = remaining.trim_start_matches(',');
            labels.insert(key.to_string(), value);
        } else {
            // Unquoted value (until comma or end)
            let end = remaining.find(',').unwrap_or(remaining.len());
            let value = remaining[..end].trim();
            labels.insert(key.to_string(), value.to_string());
            remaining = if end < remaining.len() {
                &remaining[end + 1..]
            } else {
                ""
            };
        }
    }

    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prometheus_metrics() {
        let input = r#"# HELP monitor_status Monitor Status (1 = UP, 0= DOWN, 2= PENDING, 3= MAINTENANCE)
# TYPE monitor_status gauge
monitor_status{monitor_name="Nextcloud",monitor_type="http",monitor_url="https://cloud.kensai.cloud",monitor_hostname="",monitor_port=""} 1
monitor_status{monitor_name="SSH",monitor_type="port",monitor_url="",monitor_hostname="ssh.kensai.cloud",monitor_port="22022"} 1
monitor_status{monitor_name="Caddy",monitor_type="docker",monitor_url="",monitor_hostname="caddy",monitor_port=""} 0
# HELP monitor_response_time Monitor Response Time (ms)
# TYPE monitor_response_time gauge
monitor_response_time{monitor_name="Nextcloud",monitor_type="http",monitor_url="https://cloud.kensai.cloud",monitor_hostname="",monitor_port=""} 145
monitor_response_time{monitor_name="SSH",monitor_type="port",monitor_url="",monitor_hostname="ssh.kensai.cloud",monitor_port="22022"} 23
# HELP monitor_cert_days_remaining len(TLS certificate  validity)
# TYPE monitor_cert_days_remaining gauge
monitor_cert_days_remaining{monitor_name="Nextcloud",monitor_type="http",monitor_url="https://cloud.kensai.cloud",monitor_hostname="",monitor_port=""} 89
# HELP monitor_cert_is_valid len(TLS certificate valid)
# TYPE monitor_cert_is_valid gauge
monitor_cert_is_valid{monitor_name="Nextcloud",monitor_type="http",monitor_url="https://cloud.kensai.cloud",monitor_hostname="",monitor_port=""} 1
"#;

        let result = parse_prometheus_metrics(input).unwrap();
        assert_eq!(result.total, 3);
        assert_eq!(result.up, 2);
        assert_eq!(result.down, 1);

        let nc = result
            .monitors
            .iter()
            .find(|m| m.name == "Nextcloud")
            .unwrap();
        assert_eq!(nc.status, 1);
        assert_eq!(nc.status_text, "up");
        assert_eq!(nc.response_time_ms, Some(145.0));
        assert_eq!(nc.cert_days_remaining, Some(89.0));
        assert_eq!(nc.cert_is_valid, Some(true));

        let caddy = result.monitors.iter().find(|m| m.name == "Caddy").unwrap();
        assert_eq!(caddy.status, 0);
        assert_eq!(caddy.status_text, "down");
        assert_eq!(caddy.monitor_type, "docker");
    }

    #[test]
    fn test_parse_labels() {
        let labels = parse_labels(
            r#"monitor_name="Test Server",monitor_type="http",monitor_url="https://example.com""#,
        );
        assert_eq!(labels.get("monitor_name").unwrap(), "Test Server");
        assert_eq!(labels.get("monitor_type").unwrap(), "http");
        assert_eq!(labels.get("monitor_url").unwrap(), "https://example.com");
    }

    #[test]
    fn test_parse_labels_escaped_quotes() {
        let labels = parse_labels(r#"monitor_name="Test \"Quoted\" Server",monitor_type="http""#);
        assert_eq!(
            labels.get("monitor_name").unwrap(),
            r#"Test "Quoted" Server"#
        );
    }
}
