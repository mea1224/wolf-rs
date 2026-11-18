// Optional vector generator, not part of the runtime or Cargo build.
// From workspace root: g++ -std=c++20 -O2 wolf-rs/crates/wolf-assets/tests/reference/wolf_aes_vector.cpp -o wolf-rs/target/wolf-aes-vector.exe
// Uses the locally retained upstream reference. See wolf-rs/THIRD_PARTY_NOTICES.md.
#include <cstring>
#include "../../../../../docs/tools/upstream/UberWolf-main/UberWolfLib/WolfCrypt/WolfAes.hpp"
#include <cstdio>

int main() {
    for (int test = 0; test < 2; ++test) {
        uint8_t password[15];
        for (int i = 0; i < 15; ++i) password[i] = test == 0 ? i : 255;
        uint8_t key[wolf::crypt::aes::ROUND_KEY_SIZE] = {};
        uint8_t data[48] = {};
        wolf::crypt::aes::initAES128(key, password, nullptr, 350);
        wolf::crypt::aes::aesCtrXCrypt(data, key, 17);
        wolf::crypt::aes::aesCtrXCrypt(data + 17, key, 31);
        for (uint8_t b : data) std::printf("%02x", b);
        std::puts("");
    }
}
