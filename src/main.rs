use anyhow::Result;
use clap::Parser;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::DeleteParams, Api, Client};
use log::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::error::Error;
use tokio::time::{interval, Duration};

/// Struct for command line arguments using clap
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Alertmanager URL to poll alerts from
    #[clap(short, long, env)]
    alertmanager_url: String,

    /// Alert name to match against the 'alertname' label
    #[clap(short, long, env)]
    alert_name: String,

    /// Interval in seconds to check for alerts
    #[clap(short, long, env, default_value_t = 60)]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct Alert {
    status: String,
    labels: Labels,
}

#[derive(Debug, Deserialize)]
struct Labels {
    pod: String,
    namespace: String,
    alertname: Option<String>, // This is optional as some alerts might not have this label
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

async fn delete_pod(pod: &str, namespace: &str) -> Result<(), Box<dyn Error>> {
    let client = Client::try_default().await?;
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

    loop {
        interval_timer.tick().await;

        info!("Checking for alerts...");

        match get_alerts(&args.alertmanager_url).await {
            Ok(alerts) => {
                for alert in alerts {
                    if alert.status == "firing" {
                        if let Some(alertname) = &alert.labels.alertname {
                            if alertname == &args.alert_name {
                                let pod = &alert.labels.pod;
                                let namespace = &alert.labels.namespace;
                                if let Err(err) = delete_pod(pod, namespace).await {
                                    error!("Failed to delete pod: {}", err);
                                }
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
