# Third-party notices

harbour-xmatic itself is Apache-2.0 (see `LICENSE`). The application
binary links a static Rust library that compiles 577 further crates into
it, and it bundles SQLite. Their terms are listed here because a binary
that carries them has to carry their notices too.

Every crate below is fetched from crates.io by exact version, recorded in
`core/Cargo.lock` with a checksum. Its source is available at
`https://crates.io/crates/<name>/<version>`; the licence text of each is
in the crate's own source tree.

## Weak-copyleft components (MPL-2.0)

These are covered by the Mozilla Public License 2.0. Its section 3.2
requires that anyone receiving this binary is told how to obtain the
source of these files: each is published unmodified at the crates.io
address above, and no file of theirs has been changed for this project.

- as_variant 1.3.0 — MPL-2.0
- async-rx 0.2.1 — MPL-2.0
- bitmaps 3.2.1 — MPL-2.0+
- eyeball 0.8.8 — MPL-2.0
- eyeball-im 0.8.0 — MPL-2.0
- eyeball-im-util 0.10.0 — MPL-2.0
- htmlescape 0.3.1 — Apache-2.0 / MIT / MPL-2.0
- imbl 6.1.0 — MPL-2.0+
- imbl-sized-chunks 0.1.3 — MPL-2.0+
- readlock 0.1.11 — MPL-2.0
- readlock-tokio 0.1.6 — MPL-2.0

## Bundled SQLite

SQLite is compiled from bundled sources (`libsqlite3-sys`) instead of
linking the device's library. SQLite itself is in the public domain.

## All linked crates

| Crate | Version | Licence |
|---|---|---|
| accessory | 2.1.0 | MIT OR Apache-2.0 |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| aead | 0.5.2 | MIT OR Apache-2.0 |
| aead | 0.6.1 | MIT OR Apache-2.0 |
| aes | 0.8.4 | MIT OR Apache-2.0 |
| aes | 0.9.3 | MIT OR Apache-2.0 |
| aes-gcm | 0.10.3 | Apache-2.0 OR MIT |
| aes-gcm | 0.11.1 | Apache-2.0 OR MIT |
| aes-keywrap | 0.9.0 | MIT |
| aho-corasick | 1.1.4 | Unlicense OR MIT |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |
| android_system_properties | 0.1.5 | MIT/Apache-2.0 |
| anyhow | 1.0.104 | MIT OR Apache-2.0 |
| anymap2 | 0.13.0 | MIT/Apache-2.0 |
| aquamarine | 0.6.0 | MIT |
| arc-swap | 1.9.2 | MIT OR Apache-2.0 |
| archery | 1.2.2 | MIT |
| arrayref | 0.3.9 | BSD-2-Clause |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| as_variant | 1.3.0 | MPL-2.0 |
| assign | 1.1.1 | MIT |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 |
| async-channel | 2.5.0 | Apache-2.0 OR MIT |
| async-compression | 0.4.42 | MIT OR Apache-2.0 |
| async-executor | 1.14.0 | Apache-2.0 OR MIT |
| async-io | 2.6.0 | Apache-2.0 OR MIT |
| async-lock | 3.4.2 | Apache-2.0 OR MIT |
| async-once-cell | 0.5.4 | MIT OR Apache-2.0 |
| async-process | 2.5.0 | Apache-2.0 OR MIT |
| async-recursion | 1.1.1 | MIT OR Apache-2.0 |
| async-rx | 0.2.1 | MPL-2.0 |
| async-signal | 0.2.14 | Apache-2.0 OR MIT |
| async-stream | 0.3.6 | MIT |
| async-stream-impl | 0.3.6 | MIT |
| async-task | 4.7.1 | Apache-2.0 OR MIT |
| async-trait | 0.1.91 | MIT OR Apache-2.0 |
| async_cell | 0.2.3 | MIT |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| aws-lc-rs | 1.17.3 | ISC AND (Apache-2.0 OR ISC) |
| aws-lc-sys | 0.43.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) |
| axum | 0.8.9 | MIT |
| axum-core | 0.5.6 | MIT |
| backon | 1.6.0 | Apache-2.0 |
| base16ct | 0.2.0 | Apache-2.0 OR MIT |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT |
| binstring | 0.1.7 | MIT |
| bitflags | 2.13.1 | MIT OR Apache-2.0 |
| bitmaps | 3.2.1 | MPL-2.0+ |
| bitpacking | 0.9.3 | MIT |
| blake2b_simd | 1.0.5 | MIT |
| blake3 | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 |
| block-padding | 0.3.3 | MIT OR Apache-2.0 |
| blocking | 1.7.0 | Apache-2.0 OR MIT |
| bon | 3.10.0 | MIT OR Apache-2.0 |
| bon-macros | 3.10.0 | MIT OR Apache-2.0 |
| bs58 | 0.5.1 | MIT/Apache-2.0 |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 |
| byteorder | 1.5.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| bytesize | 2.4.2 | Apache-2.0 |
| cbc | 0.1.2 | MIT OR Apache-2.0 |
| cc | 1.3.0 | MIT OR Apache-2.0 |
| census | 0.4.2 | MIT |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 |
| cfg_aliases | 0.2.2 | MIT |
| chacha20 | 0.10.1 | MIT OR Apache-2.0 |
| chacha20 | 0.9.1 | Apache-2.0 OR MIT |
| chacha20poly1305 | 0.10.1 | Apache-2.0 OR MIT |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| cipher | 0.4.4 | MIT OR Apache-2.0 |
| cipher | 0.5.2 | MIT OR Apache-2.0 |
| cmake | 0.1.58 | MIT OR Apache-2.0 |
| cmov | 0.5.4 | Apache-2.0 OR MIT |
| coarsetime | 0.1.37 | BSD-2-Clause |
| combine | 4.6.7 | MIT |
| compression-codecs | 0.4.38 | MIT OR Apache-2.0 |
| compression-core | 0.4.32 | MIT OR Apache-2.0 |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT |
| const-oid | 0.10.2 | Apache-2.0 OR MIT |
| const-oid | 0.9.6 | Apache-2.0 OR MIT |
| const_panic | 0.2.15 | Zlib |
| constant_time_eq | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 |
| cpubits | 0.1.1 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| cpufeatures | 0.3.0 | MIT OR Apache-2.0 |
| crc32fast | 1.5.0 | MIT OR Apache-2.0 |
| crossbeam-channel | 0.5.16 | MIT OR Apache-2.0 |
| crossbeam-deque | 0.8.7 | MIT OR Apache-2.0 |
| crossbeam-epoch | 0.9.20 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.22 | MIT OR Apache-2.0 |
| crunchy | 0.2.4 | MIT |
| crypto-bigint | 0.5.5 | Apache-2.0 OR MIT |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 |
| ct-codecs | 1.1.7 | MIT |
| ctr | 0.10.1 | MIT OR Apache-2.0 |
| ctr | 0.9.2 | MIT OR Apache-2.0 |
| ctutils | 0.4.2 | Apache-2.0 OR MIT |
| curve25519-dalek | 4.1.3 | BSD-3-Clause |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 |
| darling | 0.24.1 | MIT |
| darling_core | 0.24.1 | MIT |
| darling_macro | 0.24.1 | MIT |
| datasketches | 0.2.0 | Apache-2.0 |
| date_header | 1.0.5 | MIT/Apache-2.0 |
| deadpool | 0.13.0 | MIT OR Apache-2.0 |
| deadpool-runtime | 0.3.1 | MIT OR Apache-2.0 |
| deadpool-sync | 0.2.0 | MIT OR Apache-2.0 |
| decancer | 3.3.3 | MIT |
| delegate-display | 3.0.0 | MIT |
| der | 0.7.10 | Apache-2.0 OR MIT |
| der | 0.8.1 | Apache-2.0 OR MIT |
| deranged | 0.5.8 | MIT OR Apache-2.0 |
| derivative | 2.2.0 | MIT/Apache-2.0 |
| derive_more | 1.0.0 | MIT |
| derive_more | 2.1.1 | MIT |
| derive_more-impl | 1.0.0 | MIT |
| derive_more-impl | 2.1.1 | MIT |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| digest | 0.11.3 | MIT OR Apache-2.0 |
| displaydoc | 0.2.6 | MIT OR Apache-2.0 |
| downcast-rs | 2.0.2 | MIT OR Apache-2.0 |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| ecdsa | 0.16.9 | Apache-2.0 OR MIT |
| ece-native | 0.4.0 | MIT OR Apache-2.0 |
| ed25519 | 2.2.3 | Apache-2.0 OR MIT |
| ed25519-compact | 2.4.2 | MIT |
| ed25519-dalek | 2.2.0 | BSD-3-Clause |
| either | 1.16.0 | MIT OR Apache-2.0 |
| elliptic-curve | 0.13.8 | Apache-2.0 OR MIT |
| emojis | 0.8.2 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| endi | 1.1.1 | MIT |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 |
| equivalent | 1.0.2 | Apache-2.0 OR MIT |
| erased-serde | 0.4.10 | MIT OR Apache-2.0 |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| event-listener | 5.4.1 | Apache-2.0 OR MIT |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT |
| eyeball | 0.8.8 | MPL-2.0 |
| eyeball-im | 0.8.0 | MPL-2.0 |
| eyeball-im-util | 0.10.0 | MPL-2.0 |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fancy_constructor | 2.1.0 | MIT OR Apache-2.0 |
| fastdivide | 0.4.2 | zlib-acknowledgement OR MIT |
| fastrand | 2.5.0 | Apache-2.0 OR MIT |
| ff | 0.13.1 | MIT/Apache-2.0 |
| fiat-crypto | 0.2.9 | MIT OR Apache-2.0 OR BSD-1-Clause |
| find-msvc-tools | 0.1.9 | MIT OR Apache-2.0 |
| flate2 | 1.1.9 | MIT OR Apache-2.0 |
| fnv | 1.0.7 | Apache-2.0 / MIT |
| foldhash | 0.1.5 | Zlib |
| foldhash | 0.2.0 | Zlib |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| fs4 | 0.13.1 | MIT OR Apache-2.0 |
| fs_extra | 1.3.0 | MIT |
| futures-channel | 0.3.33 | MIT OR Apache-2.0 |
| futures-core | 0.3.33 | MIT OR Apache-2.0 |
| futures-executor | 0.3.33 | MIT OR Apache-2.0 |
| futures-io | 0.3.33 | MIT OR Apache-2.0 |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT |
| futures-macro | 0.3.33 | MIT OR Apache-2.0 |
| futures-sink | 0.3.33 | MIT OR Apache-2.0 |
| futures-task | 0.3.33 | MIT OR Apache-2.0 |
| futures-util | 0.3.33 | MIT OR Apache-2.0 |
| fuzzy-matcher | 0.3.7 | MIT |
| generator | 0.8.9 | MIT/Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| ghash | 0.5.1 | Apache-2.0 OR MIT |
| ghash | 0.6.0 | Apache-2.0 OR MIT |
| gloo-timers | 0.3.0 | MIT OR Apache-2.0 |
| gloo-utils | 0.2.0 | MIT OR Apache-2.0 |
| group | 0.13.0 | MIT/Apache-2.0 |
| growable-bloom-filter | 2.1.1 | MIT |
| h2 | 0.4.15 | MIT |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hashlink | 0.10.0 | MIT OR Apache-2.0 |
| hermit-abi | 0.5.2 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| hkdf | 0.12.4 | MIT OR Apache-2.0 |
| hmac | 0.12.1 | MIT OR Apache-2.0 |
| hmac-sha1-compact | 1.1.7 | ISC |
| hmac-sha256 | 1.1.14 | ISC |
| hmac-sha512 | 1.1.12 | ISC |
| html5ever | 0.39.0 | MIT OR Apache-2.0 |
| htmlescape | 0.3.1 | Apache-2.0 / MIT / MPL-2.0 |
| http | 1.4.2 | MIT OR Apache-2.0 |
| http-body | 1.1.0 | MIT |
| http-body-util | 0.1.4 | MIT |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| httpdate | 1.0.3 | MIT OR Apache-2.0 |
| hybrid-array | 0.4.14 | MIT OR Apache-2.0 |
| hyper | 1.11.0 | MIT |
| hyper-rustls | 0.27.9 | Apache-2.0 OR ISC OR MIT |
| hyper-util | 0.1.20 | MIT |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 |
| icu_collections | 2.2.0 | Unicode-3.0 |
| icu_locale_core | 2.2.0 | Unicode-3.0 |
| icu_normalizer | 2.2.0 | Unicode-3.0 |
| icu_normalizer_data | 2.2.0 | Unicode-3.0 |
| icu_properties | 2.2.0 | Unicode-3.0 |
| icu_properties_data | 2.2.0 | Unicode-3.0 |
| icu_provider | 2.2.0 | Unicode-3.0 |
| ident_case | 1.0.1 | MIT/Apache-2.0 |
| idna | 1.1.0 | MIT OR Apache-2.0 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT |
| imbl | 6.1.0 | MPL-2.0+ |
| imbl-sized-chunks | 0.1.3 | MPL-2.0+ |
| impartial-ord | 1.0.6 | MIT |
| include_dir | 0.7.4 | MIT |
| include_dir_macros | 0.7.4 | MIT |
| indexmap | 2.14.0 | Apache-2.0 OR MIT |
| inout | 0.1.4 | MIT OR Apache-2.0 |
| inout | 0.2.2 | MIT OR Apache-2.0 |
| inventory | 0.3.24 | MIT OR Apache-2.0 |
| ipnet | 2.12.0 | MIT OR Apache-2.0 |
| itertools | 0.10.5 | MIT/Apache-2.0 |
| itertools | 0.14.0 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| jni | 0.22.4 | MIT OR Apache-2.0 |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 |
| jobserver | 0.1.35 | MIT OR Apache-2.0 |
| js-sys | 0.3.103 | MIT OR Apache-2.0 |
| js_int | 0.2.2 | MIT |
| js_option | 0.2.0 | MIT |
| jwt-simple | 0.12.17 | ISC |
| k256 | 0.13.4 | Apache-2.0 OR MIT |
| keccak | 0.2.2 | Apache-2.0 OR MIT |
| konst | 0.4.3 | Zlib |
| language-tags | 0.3.2 | MIT/Apache-2.0 |
| lazy_static | 1.5.0 | MIT OR Apache-2.0 |
| levenshtein_automata | 0.2.1 | MIT |
| libc | 0.2.189 | MIT OR Apache-2.0 |
| libm | 0.2.16 | MIT |
| libsqlite3-sys | 0.35.0 | MIT |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| litemap | 0.8.2 | Unicode-3.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| log | 0.4.33 | MIT OR Apache-2.0 |
| loom | 0.7.2 | MIT |
| lru | 0.16.4 | MIT |
| lru-slab | 0.1.2 | MIT OR Apache-2.0 OR Zlib |
| lz4_flex | 0.13.1 | MIT |
| macroific | 2.0.0 | Apache-2.0 |
| macroific_attr_parse | 2.0.0 | Apache-2.0 |
| macroific_core | 2.0.0 | Apache-2.0 |
| macroific_macro | 2.0.0 | Apache-2.0 |
| maplit | 1.0.2 | MIT/Apache-2.0 |
| markup5ever | 0.39.0 | MIT OR Apache-2.0 |
| matchers | 0.2.0 | MIT |
| matchit | 0.8.4 | MIT AND BSD-3-Clause |
| matrix-pickle | 0.2.3 | Apache-2.0 |
| matrix-pickle-derive | 0.2.3 | Apache-2.0 |
| matrix-sdk | 0.18.0 | Apache-2.0 |
| matrix-sdk-base | 0.18.0 | Apache-2.0 |
| matrix-sdk-common | 0.18.0 | Apache-2.0 |
| matrix-sdk-crypto | 0.18.0 | Apache-2.0 |
| matrix-sdk-indexeddb | 0.18.0 | Apache-2.0 |
| matrix-sdk-search | 0.18.0 | Apache-2.0 |
| matrix-sdk-sqlite | 0.18.0 | Apache-2.0 |
| matrix-sdk-store-encryption | 0.18.0 | Apache-2.0 |
| matrix-sdk-ui | 0.18.0 | Apache-2.0 |
| matrix_indexed_db_futures | 0.7.0 | MIT |
| matrix_indexed_db_futures_macros_internal | 1.0.0 | MIT |
| measure_time | 0.9.0 | MIT |
| memchr | 2.8.3 | Unlicense OR MIT |
| memmap2 | 0.9.11 | MIT OR Apache-2.0 |
| memoffset | 0.9.1 | MIT |
| mime | 0.3.17 | MIT OR Apache-2.0 |
| mime2ext | 0.1.54 | MIT |
| minimal-lexical | 0.2.1 | MIT/Apache-2.0 |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| mio | 1.2.2 | MIT |
| ml-dsa | 0.1.1 | Apache-2.0 OR MIT |
| module-lattice | 0.2.3 | Apache-2.0 OR MIT |
| murmurhash32 | 0.3.1 | MIT |
| new_debug_unreachable | 1.0.6 | MIT |
| nom | 7.1.3 | MIT |
| nu-ansi-term | 0.50.3 | MIT |
| num-bigint-dig | 0.8.6 | MIT/Apache-2.0 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 |
| num-integer | 0.1.47 | MIT OR Apache-2.0 |
| num-iter | 0.1.46 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| num_cpus | 1.17.0 | MIT OR Apache-2.0 |
| oauth2 | 5.0.0 | MIT OR Apache-2.0 |
| oauth2-reqwest | 0.1.0-alpha.3 | MIT |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| oneshot | 0.1.13 | MIT OR Apache-2.0 |
| opaque-debug | 0.3.1 | MIT OR Apache-2.0 |
| openssl-probe | 0.2.1 | MIT OR Apache-2.0 |
| ordered-float | 5.5.0 | MIT |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 |
| ownedbytes | 0.9.0 | MIT |
| p256 | 0.13.2 | Apache-2.0 OR MIT |
| p384 | 0.13.1 | Apache-2.0 OR MIT |
| parking | 2.2.1 | Apache-2.0 OR MIT |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| pbkdf2 | 0.12.2 | MIT OR Apache-2.0 |
| pem-rfc7468 | 0.7.0 | Apache-2.0 OR MIT |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| phf | 0.13.1 | MIT |
| phf_codegen | 0.13.1 | MIT |
| phf_generator | 0.13.1 | MIT |
| phf_shared | 0.13.1 | MIT |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| piper | 0.2.5 | MIT OR Apache-2.0 |
| pkcs1 | 0.7.5 | Apache-2.0 OR MIT |
| pkcs8 | 0.10.2 | Apache-2.0 OR MIT |
| pkcs8 | 0.11.0 | Apache-2.0 OR MIT |
| pkg-config | 0.3.33 | MIT OR Apache-2.0 |
| polling | 3.11.0 | Apache-2.0 OR MIT |
| poly1305 | 0.8.0 | Apache-2.0 OR MIT |
| polyval | 0.6.2 | Apache-2.0 OR MIT |
| polyval | 0.7.3 | Apache-2.0 OR MIT |
| potential_utf | 0.1.5 | Unicode-3.0 |
| powerfmt | 0.2.0 | MIT OR Apache-2.0 |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 |
| precomputed-hash | 0.1.1 | MIT |
| prettyplease | 0.3.0 | MIT OR Apache-2.0 |
| primeorder | 0.13.6 | Apache-2.0 OR MIT |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 |
| proc-macro-error-attr2 | 2.0.0 | MIT OR Apache-2.0 |
| proc-macro-error2 | 2.0.1 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| prost | 0.14.4 | Apache-2.0 |
| prost-derive | 0.14.4 | Apache-2.0 |
| quinn | 0.11.11 | MIT OR Apache-2.0 |
| quinn-proto | 0.11.16 | MIT OR Apache-2.0 |
| quinn-udp | 0.5.15 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| rand | 0.10.2 | MIT OR Apache-2.0 |
| rand | 0.8.7 | MIT OR Apache-2.0 |
| rand | 0.9.5 | MIT OR Apache-2.0 |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 |
| rand_core | 0.6.4 | MIT OR Apache-2.0 |
| rand_core | 0.9.5 | MIT OR Apache-2.0 |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 |
| rand_xoshiro | 0.7.0 | MIT OR Apache-2.0 |
| rayon | 1.12.0 | MIT OR Apache-2.0 |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 |
| readlock | 0.1.11 | MPL-2.0 |
| readlock-tokio | 0.1.6 | MPL-2.0 |
| redox_syscall | 0.5.18 | MIT |
| regex | 1.13.1 | MIT OR Apache-2.0 |
| regex-automata | 0.4.16 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 |
| reqwest | 0.13.4 | MIT OR Apache-2.0 |
| rfc6979 | 0.4.0 | Apache-2.0 OR MIT |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rmp | 0.8.15 | MIT |
| rmp-serde | 1.3.1 | MIT |
| rsa | 0.9.10 | MIT OR Apache-2.0 |
| ruma | 0.16.0 | MIT |
| ruma-client-api | 0.24.0 | MIT |
| ruma-common | 0.19.0 | MIT |
| ruma-events | 0.34.0 | MIT |
| ruma-html | 0.8.0 | MIT |
| ruma-identifiers-validation | 0.12.1 | MIT |
| ruma-macros | 0.19.0 | MIT |
| rusqlite | 0.37.0 | MIT |
| rust-stemmers | 1.2.0 | MIT/BSD-3-Clause |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 |
| rustix | 1.1.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustls | 0.23.42 | Apache-2.0 OR ISC OR MIT |
| rustls-native-certs | 0.8.4 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-platform-verifier | 0.7.0 | MIT OR Apache-2.0 |
| rustls-platform-verifier-android | 0.1.1 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.13 | ISC |
| rustversion | 1.0.23 | MIT OR Apache-2.0 |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| schannel | 0.1.29 | MIT |
| scoped-tls | 1.0.1 | MIT/Apache-2.0 |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| sealed | 0.6.0 | MIT OR Apache-2.0 |
| sec1 | 0.7.3 | Apache-2.0 OR MIT |
| security-framework | 3.7.0 | MIT OR Apache-2.0 |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 |
| semver | 1.0.28 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde-wasm-bindgen | 0.6.5 | MIT |
| serde_bytes | 0.11.19 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_html_form | 0.4.1 | MIT |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 |
| serde_spanned | 1.1.1 | MIT OR Apache-2.0 |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| shake | 0.1.0 | MIT OR Apache-2.0 |
| sharded-slab | 0.1.7 | MIT |
| shlex | 2.0.1 | MIT OR Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| signature | 2.2.0 | Apache-2.0 OR MIT |
| signature | 3.0.0 | Apache-2.0 OR MIT |
| simd-adler32 | 0.3.10 | MIT |
| simd_cesu8 | 1.2.0 | Apache-2.0 OR MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| siphasher | 1.0.3 | MIT/Apache-2.0 |
| sketches-ddsketch | 0.4.1 | Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.15.2 | MIT OR Apache-2.0 |
| socket2 | 0.6.5 | MIT OR Apache-2.0 |
| spin | 0.9.9 | MIT |
| spki | 0.7.3 | Apache-2.0 OR MIT |
| spki | 0.8.0 | Apache-2.0 OR MIT |
| sponge-cursor | 0.1.0 | MIT OR Apache-2.0 |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| string_cache | 0.9.0 | MIT OR Apache-2.0 |
| string_cache_codegen | 0.6.1 | MIT OR Apache-2.0 |
| strsim | 0.11.1 | MIT |
| subtle | 2.6.1 | BSD-3-Clause |
| superboring | 0.1.14 | ISC |
| syn | 1.0.109 | MIT OR Apache-2.0 |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.3 | MIT OR Apache-2.0 |
| sync_wrapper | 1.0.2 | Apache-2.0 |
| synstructure | 0.13.2 | MIT |
| tantivy | 0.26.1 | MIT |
| tantivy-bitpacker | 0.10.0 | MIT |
| tantivy-columnar | 0.7.0 | MIT |
| tantivy-common | 0.11.0 | MIT |
| tantivy-fst | 0.5.0 | Unlicense/MIT |
| tantivy-query-grammar | 0.26.0 | MIT |
| tantivy-sstable | 0.7.0 | MIT |
| tantivy-stacker | 0.7.0 | MIT |
| tantivy-tokenizer-api | 0.7.0 | MIT |
| tempfile | 3.27.0 | MIT OR Apache-2.0 |
| tendril | 0.5.1 | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| thiserror | 2.0.19 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.19 | MIT OR Apache-2.0 |
| thread_local | 1.1.10 | MIT OR Apache-2.0 |
| time | 0.3.54 | MIT OR Apache-2.0 |
| time-core | 0.1.9 | MIT OR Apache-2.0 |
| time-macros | 0.2.32 | MIT OR Apache-2.0 |
| tinystr | 0.8.3 | Unicode-3.0 |
| tinyvec | 1.12.0 | Zlib OR Apache-2.0 OR MIT |
| tinyvec_macros | 0.1.1 | MIT OR Apache-2.0 OR Zlib |
| tokio | 1.53.1 | MIT |
| tokio-macros | 2.7.1 | MIT |
| tokio-rustls | 0.26.4 | MIT OR Apache-2.0 |
| tokio-stream | 0.1.19 | MIT |
| tokio-util | 0.7.19 | MIT |
| toml | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.25.13+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_parser | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| tower | 0.5.3 | MIT |
| tower-http | 0.6.11 | MIT |
| tower-layer | 0.3.3 | MIT |
| tower-service | 0.3.3 | MIT |
| tracing | 0.1.44 | MIT |
| tracing-attributes | 0.1.31 | MIT |
| tracing-core | 0.1.36 | MIT |
| tracing-log | 0.2.0 | MIT |
| tracing-subscriber | 0.3.23 | MIT |
| try-lock | 0.2.5 | MIT |
| typeid | 1.0.3 | MIT OR Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| typetag | 0.2.23 | MIT OR Apache-2.0 |
| typetag-impl | 0.2.23 | MIT OR Apache-2.0 |
| typewit | 1.15.2 | Zlib |
| uds_windows | 1.2.1 | MIT |
| ulid | 1.2.1 | MIT |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-normalization | 0.1.25 | MIT OR Apache-2.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| unicode-xid | 0.2.6 | MIT OR Apache-2.0 |
| unifiedpush | 0.1.0 | Apache-2.0 |
| unifiedpush_storage | 0.1.0 | Apache-2.0 |
| universal-hash | 0.5.1 | MIT OR Apache-2.0 |
| universal-hash | 0.6.1 | MIT OR Apache-2.0 |
| untrusted | 0.9.0 | ISC |
| url | 2.5.8 | MIT OR Apache-2.0 |
| urlencoding | 2.1.3 | MIT |
| utf8-ranges | 1.0.5 | Unlicense/MIT |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT |
| uuid | 1.24.0 | Apache-2.0 OR MIT |
| valuable | 0.1.1 | MIT |
| vcpkg | 0.2.15 | MIT/Apache-2.0 |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| vodozemac | 0.10.0 | Apache-2.0 |
| walkdir | 2.5.0 | Unlicense/MIT |
| want | 0.3.1 | MIT |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| wasix | 0.13.2 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| wasm-bindgen | 0.2.126 | MIT OR Apache-2.0 |
| wasm-bindgen-futures | 0.4.76 | MIT OR Apache-2.0 |
| wasm-bindgen-macro | 0.2.126 | MIT OR Apache-2.0 |
| wasm-bindgen-macro-support | 0.2.126 | MIT OR Apache-2.0 |
| wasm-bindgen-shared | 0.2.126 | MIT OR Apache-2.0 |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 |
| wasm_evt_listener | 0.1.0 | MIT |
| web-push-native | 0.4.0 | MIT OR Apache-2.0 |
| web-sys | 0.3.103 | MIT OR Apache-2.0 |
| web-time | 1.1.0 | MIT OR Apache-2.0 |
| web_atoms | 0.2.5 | MIT OR Apache-2.0 |
| webpki-root-certs | 1.0.9 | CDLA-Permissive-2.0 |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 |
| wildmatch | 2.6.1 | MIT |
| winapi | 0.3.9 | MIT/Apache-2.0 |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 |
| winapi-util | 0.1.11 | Unlicense OR MIT |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| winnow | 1.0.4 | MIT |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| writeable | 0.6.3 | Unicode-3.0 |
| x25519-dalek | 2.0.1 | BSD-3-Clause |
| xxhash-rust | 0.8.18 | BSL-1.0 |
| yoke | 0.8.3 | Unicode-3.0 |
| yoke-derive | 0.8.2 | Unicode-3.0 |
| zbus | 5.19.0 | MIT |
| zbus_macros | 5.19.0 | MIT |
| zbus_names | 4.3.4 | MIT |
| zcheapstr | 1.1.0 | MIT |
| zerocopy | 0.8.55 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.55 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.7 | Unicode-3.0 |
| zeroize | 1.9.0 | Apache-2.0 OR MIT |
| zeroize_derive | 1.5.0 | Apache-2.0 OR MIT |
| zerotrie | 0.2.4 | Unicode-3.0 |
| zerovec | 0.11.6 | Unicode-3.0 |
| zerovec-derive | 0.11.3 | Unicode-3.0 |
| zmij | 1.0.23 | MIT |
| zstd | 0.13.3 | MIT |
| zstd-safe | 7.2.4 | MIT OR Apache-2.0 |
| zstd-sys | 2.0.16+zstd.1.5.7 | MIT/Apache-2.0 |
| zvariant | 5.15.0 | MIT |
| zvariant_derive | 5.15.0 | MIT |
| zvariant_utils | 4.2.0 | MIT |

Generated from `core/Cargo.lock` and the crates' own `Cargo.toml`
licence fields (`tools/third-party.py`). Regenerate it whenever a
dependency is added or a version changes.
