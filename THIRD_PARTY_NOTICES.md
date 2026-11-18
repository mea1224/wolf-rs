# Third-party notices

The WOLF350 archive codec in `crates/wolf-assets/src/wolf350/` is a Rust
port/adaptation of algorithms retained in the UberWolf source snapshot at
`../docs/tools/upstream/UberWolf-main/`. Its Sinflower cryptographic code and
the DXLib archive/compression references have separate provenance below.

## Sinflower / UberWolf cryptography

The upstream root `LICENSE` and these source headers contain the MIT License:

Copyright (c) 2023 Sinflower

Copyright (c) 2024 Sinflower (`WolfCrypt.hpp`)

Copyright (c) 2026 Sinflower (`WolfAes.hpp`, `WolfCryptUtils.hpp`, `WolfRng.hpp`)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Takumi Yamada / DXLib archive and compression

The DXArchive LZ and Huffman readers in `compression.rs` were implemented with
reference to `3rdParty/DXLib/DXArchive.cpp` and `3rdParty/DXLib/Huffman.cpp`
in that snapshot. Their headers credit 山田 巧 (Takumi Yamada), with creation
dates 2003/09/11 and 2018/12/16 respectively; `DXArchive.cpp` identifies version
1.02. The retained DXLib directory contains no accompanying license grant.
UberWolf's root MIT notice does not establish a license for these separately
authored third-party files. Verification of applicable DXLib redistribution
terms remains unresolved; this notice does not assign them an MIT license.

## Source identity

The snapshot was downloaded from `https://github.com/Sinflower/UberWolf`.
These SHA-256 hashes identify the actual local reference files, independently
of the reference executable's release label:

| Path relative to the snapshot | SHA-256 |
| --- | --- |
| `LICENSE` | `5d12ed636533686d9733a000c74f93de43ff158bdd6354c13decb61a2f3a7967` |
| `UberWolfLib/WolfCrypt/WolfCrypt.hpp` | `beb85af07b33d42145c5a719e101c963243e07a88b51b2a7afcef2aa9055aa94` |
| `UberWolfLib/WolfCrypt/WolfAes.hpp` | `3394c10fc651b425d8226d1e819aae6a721922337dfbda98238473b8d4e7504e` |
| `UberWolfLib/WolfCrypt/WolfCryptUtils.hpp` | `6f3be39198de6c04849c90f25a4924dfa52d189543eaea844e6ccd7aad706fb3` |
| `UberWolfLib/WolfCrypt/WolfRng.hpp` | `8324d84f7155589a4abe008150f0350e3a03984e6596bffc816388b0891435e7` |
| `3rdParty/DXLib/DXArchive.cpp` | `0103439cd52cd85ce86b05df167c54a49f9f545b994d9968da59535db4af2826` |
| `3rdParty/DXLib/Huffman.cpp` | `f449a01b8c5dcf6a1f539dbdb77d564fbdc96ba74e2e22b8d28a95392220ee9b` |
