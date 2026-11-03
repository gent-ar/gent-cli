#pragma once
#include <cstdint>
#include <cstddef>
#include <stdexcept>
namespace gent_model_protocol {
inline uint32_t required(int32_t reported, uint32_t limit) {
    if (reported >= 0) throw std::runtime_error("required-size-sign");
    const auto value = -static_cast<int64_t>(reported);
    if (value < 1 || value > limit) throw std::runtime_error("required-size-bound");
    return static_cast<uint32_t>(value);
}
inline void piece_size(int32_t returned, std::size_t capacity) {
    if (returned < 0 || static_cast<uint64_t>(returned) > capacity)
        throw std::runtime_error("token-piece-bound");
}
}
