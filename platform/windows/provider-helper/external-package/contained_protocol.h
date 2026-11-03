#pragma once
#include <array>
#include <cstdint>

namespace contained_protocol {
constexpr uint32_t max_data = 32768;
constexpr uint32_t end = 0;
constexpr uint32_t abort = UINT32_MAX;
constexpr uint32_t ack = UINT32_MAX - 1;
inline std::array<char, 4> encode(uint32_t value) {
    return {static_cast<char>(value), static_cast<char>(value >> 8),
        static_cast<char>(value >> 16), static_cast<char>(value >> 24)};
}
inline uint32_t decode(const std::array<unsigned char, 4>& bytes) {
    return static_cast<uint32_t>(bytes[0]) | (static_cast<uint32_t>(bytes[1]) << 8)
        | (static_cast<uint32_t>(bytes[2]) << 16) | (static_cast<uint32_t>(bytes[3]) << 24);
}
inline bool data(uint32_t value) { return value && value <= max_data; }
}
