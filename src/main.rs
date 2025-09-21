use anyhow::Result;
use clap::Parser;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::DeleteParams, Api, Client};
use kube_leader_election::{LeaseLock, LeaseLockParams};
use log::{error, info, warn};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::{error::Error, process};
use tokio::time::{interval, Duration};

/// Struct for command line arguments using clap
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Alertmanager URL to poll alerts from
    #[clap(short, long, env)]
    alertmanager_url: String,

    /// Alert name to match against the 'alertname' label
    #[clap(short, long, env, value_delimiter = ',')]
    alert_names: Vec<String>,

    /// Interval in seconds to check for alerts
    #[clap(short, long, env, default_value_t = 60)]
    interval: u64,

    /// Pod name for leader election
    #[clap(long, env)]
    pod_name: String,

    /// Name for lease
    #[clap(short, long, env, default_value = "alert-deleter")]
    lease_name: String,

    /// Duration for lease
    #[clap(short, long, env, default_value_t = 10)]
    lease_secs: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Alert {
    fingerprint: String,
    status: AlertStatus,
    labels: Labels,
}

#[derive(Debug, Deserialize, Serialize)]
struct AlertStatus {
    state: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Labels {
    alertname: String,
    pod: Option<String>,         // Pod might be missing in some alerts
    namespace: Option<String>,   // Namespace might be missing in some alerts
    action: Option<String>,      // Action to take: delete_pod or webhook
    webhook_url: Option<String>, // Webhook URL for this specific alert
}

async fn get_alerts(alertmanager_url: &str) -> Result<Vec<Alert>, Box<dyn Error>> {
    let http_client = HttpClient::new();
    let resp = http_client
        .get(alertmanager_url)
        .send()
        .await?
        .json::<Vec<Alert>>()
        .await?;

    Ok(resp)
}

async fn delete_pod(client: Client, pod: &str, namespace: &str) -> Result<(), Box<dyn Error>> {
    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let dp = DeleteParams::default();
    pods.delete(pod, &dp).await?;
    info!("Deleted pod {} in namespace {}", pod, namespace);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    simple_logger::init_with_level(log::Level::Info)?;
    // Parse command line arguments
    let args = Args::parse();
    // Interval to poll for alerts
    let mut interval_timer = interval(Duration::from_secs(args.interval));

    let client = Client::try_default().await?;
    let namespace = client.default_namespace();
    let leadership = LeaseLock::new(
        client.clone(),
        namespace,
        LeaseLockParams {
            holder_id: args.pod_name,
            lease_name: args.lease_name,
            lease_ttl: Duration::from_secs(args.lease_secs),
        },
    );

    info!("waiting for lock...");
    loop {
        let lease = leadership.try_acquire_or_renew().await?;
        if lease.acquired_lease {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    info!("acquired lock!");

    // start a background thread to see if we're still leader
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let lease = match leadership.try_acquire_or_renew().await {
                Result::Ok(l) => l,
                Err(e) => {
                    warn!("background lease error: {}", e);
                    continue;
                }
            };
            if !lease.acquired_lease {
                info!("lost lease, exiting...");
                process::exit(1);
            }
        }
    });

    // main loop
    loop {
        interval_timer.tick().await;
        info!("Checking for alerts...");
        match get_alerts(&args.alertmanager_url).await {
            Ok(alerts) => {
                for alert in alerts {
                    // Only check for alerts that match the provided alert name
                    if args.alert_names.contains(&alert.labels.alertname)
                        && alert.status.state == "active"
                    {
                        // Check for action label - default to delete_pod if not specified
                        let action = alert.labels.action.as_deref().unwrap_or("delete_pod");

                        match action {
                            "delete_pod" => {
                                if let (Some(pod), Some(namespace)) =
                                    (&alert.labels.pod, &alert.labels.namespace)
                                {
                                    if let Err(err) =
                                        delete_pod(client.clone(), pod, namespace).await
                                    {
                                        error!("Failed to delete pod: {}", err);
                                    }
                                } else {
                                    error!(
                                        "Alert {} is missing pod or namespace",
                                        alert.fingerprint
                                    );
                                }
                            }
                            "webhook" => {
                                // Get webhook URL from alert label
                                if let Some(url) = &alert.labels.webhook_url {
                                    // Send webhook with alert data
                                    let client = HttpClient::new();
                                    let resp = client.post(url).json(&alert).send().await;
                                    match resp {
                                        Ok(_) => {
                                            info!("Sent webhook for alert {}", alert.fingerprint)
                                        }
                                        Err(err) => error!("Failed to send webhook: {}", err),
                                    }
                                } else {
                                    error!(
                                        "No webhook URL specified in alert {}",
                                        alert.fingerprint
                                    );
                                }
                            }
                            _ => {
                                // Unknown action, log and ignore
                                warn!("Unknown action '{}' in alert {}", action, alert.fingerprint);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                error!("Failed to get alerts: {}", err);
            }
        }
    }
}
