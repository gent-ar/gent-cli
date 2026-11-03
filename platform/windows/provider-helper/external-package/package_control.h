#pragma once
#include <windows.h>
#include <cstdint>
#include <string>
#include <vector>
#include "package_peer.h"
namespace external_bridge {
uint16_t start_package_control(HANDLE peer, const PackageGrant& grant,
    const std::wstring& descriptor_path, const std::string& nonce,
    const std::vector<HANDLE>& data_channels, const std::string& run_bearer);
}
