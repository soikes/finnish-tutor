output "server_ip" {
  value       = hcloud_server.finnish_tutor.ipv4_address
  description = "Public IPv4 address of the server"
}

output "server_ipv6" {
  value       = hcloud_server.finnish_tutor.ipv6_address
  description = "Public IPv6 address of the server"
}
