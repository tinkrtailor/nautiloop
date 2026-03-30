# Example: Hetzner VPS + Tailscale + Nemo
#
# Provisions a Hetzner server with:
# - Hetzner firewall (no public SSH, no public 8080)
# - Tailscale for private access (SSH + API)
# - Optional public HTTPS via domain + cert-manager
#
# The Nemo module gets the Tailscale IP, not the public IP.
# Engineers reach the API at http://nemo:8080 (Tailscale MagicDNS).
#
# This example uses Tailscale. If you use a different VPN or want public
# access, pass the appropriate IP to server_ip and manage firewall rules
# in your own terraform.
#
# Usage:
#   cd terraform/examples/hetzner
#   terraform init
#   terraform apply

terraform {
  required_version = ">= 1.5"

  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.45"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.25"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.12"
    }
  }
}

provider "hcloud" {
  token = var.hetzner_api_token
}

provider "kubernetes" {
  config_path = "${path.module}/../../modules/nemo/.state/kubeconfig.yaml"
}

provider "helm" {
  kubernetes {
    config_path = "${path.module}/../../modules/nemo/.state/kubeconfig.yaml"
  }
}

# --- Hetzner firewall: no public SSH, no public 8080 ---

resource "hcloud_firewall" "nemo" {
  name = "nemo-${var.server_location}"

  # HTTP (only if domain is set for public HTTPS)
  dynamic "rule" {
    for_each = var.domain != null ? [1] : []
    content {
      description = "HTTP (ACME + redirect)"
      direction   = "in"
      protocol    = "tcp"
      port        = "80"
      source_ips  = ["0.0.0.0/0", "::/0"]
    }
  }

  # HTTPS (only if domain is set)
  dynamic "rule" {
    for_each = var.domain != null ? [1] : []
    content {
      description = "HTTPS"
      direction   = "in"
      protocol    = "tcp"
      port        = "443"
      source_ips  = ["0.0.0.0/0", "::/0"]
    }
  }

  # Tailscale direct connections (WireGuard UDP)
  rule {
    description = "Tailscale"
    direction   = "in"
    protocol    = "udp"
    port        = "41641"
    source_ips  = ["0.0.0.0/0", "::/0"]
  }

  # Outbound (unrestricted)
  rule {
    description     = "Outbound TCP"
    direction       = "out"
    protocol        = "tcp"
    port            = "any"
    destination_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    description     = "Outbound UDP"
    direction       = "out"
    protocol        = "udp"
    port            = "any"
    destination_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    description     = "Outbound ICMP"
    direction       = "out"
    protocol        = "icmp"
    source_ips      = []
    destination_ips = ["0.0.0.0/0", "::/0"]
  }
}

# --- SSH key (for Hetzner initial cloud-init access) ---

resource "hcloud_ssh_key" "nemo" {
  count      = length(var.ssh_public_keys)
  name       = "nemo-${count.index}"
  public_key = var.ssh_public_keys[count.index]
}

# --- Server ---

resource "hcloud_server" "nemo" {
  name        = "nemo-${var.server_location}"
  server_type = var.server_type
  location    = var.server_location
  image       = "ubuntu-22.04"
  ssh_keys    = hcloud_ssh_key.nemo[*].id

  firewall_ids = [hcloud_firewall.nemo.id]

  labels = { app = "nemo" }

  user_data = templatefile("${path.module}/templates/cloud-init.yaml", {
    tailscale_auth_key = var.tailscale_auth_key
    hostname           = "nemo"
  })
}

# --- Wait for Tailscale to come up, then get the Tailscale IP ---

resource "null_resource" "tailscale_wait" {
  depends_on = [hcloud_server.nemo]

  connection {
    type        = "ssh"
    host        = hcloud_server.nemo.ipv4_address
    user        = "root"
    private_key = file(pathexpand(var.ssh_private_key_path))
  }

  provisioner "remote-exec" {
    inline = [
      "cloud-init status --wait || true",
      "TRIES=0; until tailscale status --json 2>/dev/null | jq -e '.Self.TailscaleIPs[0]' >/dev/null 2>&1 || [ $TRIES -ge 60 ]; do sleep 2; TRIES=$((TRIES+1)); done",
      "tailscale status --json | jq -r '.Self.TailscaleIPs[0]' > /tmp/tailscale_ip",
    ]
  }
}

data "external" "tailscale_ip" {
  depends_on = [null_resource.tailscale_wait]

  program = ["bash", "-c", <<-EOT
    IP=$(ssh -o StrictHostKeyChecking=accept-new \
      -o UserKnownHostsFile=/dev/null \
      -i ${pathexpand(var.ssh_private_key_path)} \
      root@${hcloud_server.nemo.ipv4_address} \
      'cat /tmp/tailscale_ip' 2>/dev/null)
    echo "{\"ip\": \"$IP\"}"
  EOT
  ]
}

# --- Install Nemo on the server via Tailscale IP ---

module "nemo" {
  source = "../../modules/nemo"

  # SSH over Tailscale — not the public IP
  server_ip       = data.external.tailscale_ip.result["ip"]
  ssh_private_key = file(pathexpand(var.ssh_private_key_path))
  ssh_user        = "root"

  git_repo_url         = var.git_repo_url
  git_host_token       = var.git_host_token
  repo_ssh_private_key = var.repo_ssh_private_key

  domain     = var.domain
  acme_email = var.acme_email

  control_plane_image = var.control_plane_image
  agent_base_image    = var.agent_base_image
  sidecar_image       = var.sidecar_image

  k3s_version          = var.k3s_version
  postgres_password    = var.postgres_password
  postgres_volume_size = var.postgres_volume_size
  ssh_known_hosts      = var.ssh_known_hosts

  image_pull_secret_dockerconfigjson = var.image_pull_secret_dockerconfigjson
}

# --- Outputs ---

output "public_ip" {
  description = "Public IP (HTTP/HTTPS only — SSH and API via Tailscale)"
  value       = hcloud_server.nemo.ipv4_address
}

output "tailscale_ip" {
  description = "Tailscale IP (use this for SSH and API access)"
  value       = data.external.tailscale_ip.result["ip"]
}

output "nemo_server_url" {
  description = "URL of the Nemo control plane"
  value       = module.nemo.server_url
}

output "ssh_command" {
  description = "SSH into the server via Tailscale"
  value       = "ssh root@nemo"
}

output "nemo_api_key" {
  description = "API key for CLI authentication"
  value       = module.nemo.api_key
  sensitive   = true
}

output "nemo_deploy_key_public" {
  description = "Public key to add as a deploy key (null if you provided your own)"
  value       = module.nemo.deploy_key_public
}

output "nemo_post_apply_instructions" {
  description = "Post-apply next steps"
  value       = module.nemo.post_apply_instructions
}
