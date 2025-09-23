# alert-actor

This tool listens to Alertmanager for alerts and performs actions on Kubernetes pods based on the alert labels. It supports two types of actions:

1. **delete_pod**: Deletes the specified pod in the given namespace
2. **webhook**: Sends an HTTP POST request with the alert data to a specified webhook URL

The tool uses leader election to ensure only one instance processes alerts at a time, and implements a cooldown mechanism to prevent repeated actions on the same alert.
