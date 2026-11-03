#pragma once
#include <cstdint>
#include <string>
int net(uint16_t port = 45679, bool ipv6 = false, const wchar_t* ipv4 = nullptr);
uint16_t gateway_port();
bool gateway_http(uint16_t port);
bool begin_inbound_probe(const std::wstring& state_root);
void finish_inbound_probe();
