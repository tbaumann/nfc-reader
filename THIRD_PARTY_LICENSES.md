# Third-party licenses

Generated from the dependency graph of the firmware that is actually flashed, not
hand-written. Regenerate with:

```sh
cd nfc-reader
cargo metadata --format-version 1 --no-default-features \
  --features board-esp32-devkit,raw-api > /tmp/meta.json
# then the inventory below is produced from /tmp/meta.json's `license` fields
```

- Scope: target `xtensa-esp32-none-elf`, features `board-esp32-devkit,raw-api`
- Crates: **233**, in 13 license expressions
- Also audited: target `xtensa-esp32s3-none-elf`, features `board-atoms3lite,raw-api`
  (236 crates) - same verdict, no copyleft in either board configuration
- Generated: 2026-10-30

## Verdict

**No copyleft dependency anywhere in the graph**: no GPL, LGPL or AGPL, in any
version, under any expression. Every crate is MIT, Apache-2.0, ISC, BSD-3-Clause,
0BSD, Unlicense, Zlib, BSL-1.0 or Unicode-3.0, all permissive, and Rust links them
all statically into the firmware image.

Consequence: **the firmware can be distributed under whatever license you choose**,
including a proprietary one. The obligations you do carry are attribution ones
(below), not source-disclosure ones.

This was a design decision, not luck: the two crates that would have covered the
NFC protocol (`iso14443`, `nfc-forum-tags`) are GPL, and the PN532 transport was
written by hand instead.

## Components outside the Cargo graph

These are in the flashed image but are not Cargo dependencies, so no `cargo`
inventory shows them:

- **ESP-IDF second-stage bootloader + partition table** - built from ESP-IDF
  (Apache-2.0), embedded in and flashed by `espflash` (MIT OR Apache-2.0,
  version 4.6.0 here). ESP-IDF ships a `NOTICE` file and contains components under
  their own terms; confirm at release time if you are distributing images.
- **Espressif radio/PHY/BT binary libraries** - `esp-wifi-sys-esp32` ships prebuilt
  `libnet80211.a`, `libpp.a`, `libphy.a`, `libcoexist.a`, `libbtdm_app.a`,
  `libcore.a` and friends, declared by that crate as MIT OR Apache-2.0. Prebuilt
  third-party binaries, not copyleft - but they are binaries, so track them.
- **ESP32 mask ROM** - in silicon, not distributed, no obligation.

## Inventory

### MIT OR Apache-2.0  (156)

- aligned 0.4.3
- allocator-api2 0.3.1
- as-slice 0.2.1
- bitfield 0.19.5
- bitfield-macros 0.19.5
- block-buffer 0.10.4
- block-buffer 0.12.1
- cc 1.10.0
- cfg-if 1.0.5
- cpufeatures 0.3.1
- critical-section 1.2.0
- crypto-common 0.1.7
- crypto-common 0.2.2
- defmt 1.2.0
- defmt-macros 1.2.0
- defmt-parser 1.1.0
- delegate 0.13.5
- digest 0.10.7
- digest 0.11.3
- docsplay 0.1.3
- docsplay-macros 0.1.2
- document-features 0.2.12
- edge-mdns 0.8.0
- edge-nal 0.7.0
- edge-nal-embassy 0.9.0
- embassy-embedded-hal 0.6.0
- embassy-executor 0.10.0
- embassy-executor-macros 0.8.0
- embassy-executor-timer-queue 0.1.0
- embassy-futures 0.1.2
- embassy-hal-internal 0.4.0
- embassy-net 0.9.1
- embassy-net-driver 0.2.0
- embassy-sync 0.6.2
- embassy-sync 0.7.2
- embassy-sync 0.8.0
- embassy-time 0.5.1
- embassy-time-driver 0.2.2
- embassy-time-queue-utils 0.3.2
- embedded-can 0.4.1
- embedded-hal 0.2.7
- embedded-hal 1.0.0
- embedded-hal-async 1.0.0
- embedded-io 0.6.1
- embedded-io 0.7.1
- embedded-io-async 0.6.1
- embedded-io-async 0.7.0
- embedded-nal 0.9.0
- embedded-nal-async 0.9.0
- embedded-storage 0.3.2
- embedded-storage-async 0.4.2
- esp-alloc 0.11.0
- esp-backtrace 0.20.0
- esp-bootloader-esp-idf 0.6.0
- esp-config 0.8.0
- esp-hal 1.2.2
- esp-hal-procmacros 0.23.0
- esp-hal-smartled 0.18.0
- esp-metadata-generated 0.5.3
- esp-phy 0.3.0
- esp-println 0.18.0
- esp-radio 1.0.0-beta.1
- esp-radio-rtos-driver 0.4.2
- esp-riscv-rt 0.15.0
- esp-rom-sys 0.1.5
- esp-rtos 0.4.0
- esp-storage 0.10.0
- esp-sync 0.3.0
- esp-wifi-sys-esp32 0.3.0
- esp-wifi-sys-esp32c2 0.3.0
- esp-wifi-sys-esp32c3 0.3.0
- esp-wifi-sys-esp32c6 0.3.0
- esp-wifi-sys-esp32h2 0.3.0
- esp-wifi-sys-esp32s2 0.3.0
- esp-wifi-sys-esp32s3 0.3.0
- esp-wifi-sys-esp32s31 0.3.0
- esp32 0.41.0
- esp32 0.42.0
- esp32c2 0.30.0
- esp32c3 0.33.0
- esp32c5 0.3.0
- esp32c6 0.24.0
- esp32c61 0.4.0
- esp32h2 0.20.0
- esp32p4 0.3.0
- esp32s2 0.32.0
- esp32s3 0.36.0
- esp32s31 0.3.0
- find-msvc-tools 0.1.15
- fugit 0.5.0
- futures-core 0.3.34
- futures-sink 0.3.34
- futures-task 0.3.34
- futures-util 0.3.34
- hash32 0.3.1
- hashbrown 0.17.1
- heapless 0.8.0
- heapless 0.9.3
- heck 0.5.0
- hybrid-array 0.4.15
- indoc 2.0.8
- itoa 1.0.18
- lazy_static 1.5.1
- libc 0.2.191
- litrs 1.0.1
- log 0.4.34
- nb 0.1.3
- nb 1.1.0
- num-derive 0.4.2
- num-traits 0.2.19
- once_cell 1.21.4
- paste 1.0.15
- proc-macro-crate 3.5.0
- proc-macro2 1.0.107
- quote 1.0.47
- rand_core 0.10.1
- rand_core 0.6.4
- rand_core 0.9.5
- regex-automata 0.4.18
- regex-syntax 0.8.11
- riscv-macros 0.3.0
- riscv-rt-macros 0.6.1
- rustversion 1.0.23
- sdio 0.5.3
- serde 1.0.229
- serde_core 1.0.229
- serde_derive 1.0.229
- serde_yaml 0.9.34+deprecated
- sha1 0.11.0
- sha2 0.11.1
- shlex 2.0.1
- smallvec 1.16.4
- smart-leds 0.4.0
- smart-leds-trait 0.3.2
- somni-expr 0.3.2
- somni-parser 0.3.1
- somni-template 0.3.2
- stable_deref_trait 1.2.1
- static_cell 2.1.1
- syn 2.0.119
- syn 3.0.9
- thiserror 2.0.21
- thiserror-impl 2.0.21
- thread_local 1.1.10
- toml_datetime 1.1.2+spec-1.1.0
- toml_edit 0.25.17+spec-1.1.0
- toml_parser 1.1.5+spec-1.1.0
- typenum 1.20.1
- ufmt-write 0.1.0
- vcell 0.1.3
- windows-link 0.2.1
- windows-result 0.4.1
- windows-sys 0.61.2
- xtensa-lx 0.13.0
- xtensa-lx-rt 0.23.0
- xtensa-lx-rt-proc-macros 0.5.0

### MIT  (35)

- block-device-driver 0.2.0
- const-default 1.0.0
- cordyceps 0.3.5
- darling 0.20.11
- darling 0.21.3
- darling 0.24.1
- darling_core 0.20.11
- darling_core 0.21.3
- darling_core 0.24.1
- darling_macro 0.20.11
- darling_macro 0.21.3
- darling_macro 0.24.1
- generic-array 0.14.7
- instability 0.3.14
- loom 0.7.2
- matchers 0.2.0
- nu-ansi-term 0.50.3
- picoserve 0.20.1
- picoserve_derive 0.2.0
- portable_atomic_enum 0.3.1
- portable_atomic_enum_macros 0.2.1
- rgb 0.8.53
- sharded-slab 0.1.7
- strsim 0.11.1
- strum 0.27.2
- strum_macros 0.27.2
- tracing 0.1.44
- tracing-attributes 0.1.31
- tracing-core 0.1.36
- tracing-log 0.2.0
- tracing-subscriber 0.3.23
- unsafe-libyaml 0.2.11
- valuable 0.1.1
- void 1.0.2
- winnow 1.0.4

### Apache-2.0 OR MIT  (9)

- autocfg 1.5.1
- equivalent 1.0.3
- indexmap 2.14.2
- pin-project 1.1.13
- pin-project-internal 1.1.13
- pin-project-lite 0.2.17
- portable-atomic 1.15.0
- portable-atomic-util 0.2.8
- semihosting 0.1.25

### MIT/Apache-2.0  (9)

- bitflags 1.3.2
- enumset 1.1.15
- enumset_derive 0.15.1
- gcd 2.3.0
- generator 0.8.11
- ident_case 1.0.1
- rlsf 0.2.3
- scoped-tls 1.0.1
- version_check 0.9.5

### Unlicense OR MIT  (8)

- aho-corasick 1.1.5
- byteorder 1.5.0
- jiff 0.2.38
- jiff-core 0.1.1
- jiff-static 0.2.38
- memchr 2.8.3
- termcolor 1.4.1
- winapi-util 0.1.11

### ISC  (4)

- riscv 0.15.0
- riscv-pac 0.2.0
- riscv-rt 0.16.0
- riscv-target-parser 0.1.3

### 0BSD  (3)

- managed 0.8.0
- smoltcp 0.13.1
- xarxa-driver 0.1.0

### BSD-3-Clause  (3)

- domain 0.12.3
- domain-macros 0.12.3
- octseq 0.6.1

### Apache-2.0/MIT  (2)

- const-sha1 0.3.0
- linked_list_allocator 0.10.6

### (MIT OR Apache-2.0) AND Unicode-3.0  (1)

- unicode-ident 1.0.27

### Apache-2.0 / MIT  (1)

- fnv 1.0.7

### Apache-2.0 OR BSL-1.0  (1)

- ryu 1.0.23

### Zlib OR Apache-2.0 OR MIT  (1)

- bytemuck 1.25.2

## License texts

Copied verbatim from the crates themselves where a registry copy was unpacked for
this target.

### MIT

As shipped by `aho-corasick 1.1.5` (LICENSE-MIT):

```
The MIT License (MIT)

Copyright (c) 2015 Andrew Gallant

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

### Apache-2.0

As shipped by `aligned 0.4.3` (LICENSE-APACHE):

```
Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS

APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### BSD-3-Clause

As shipped by `domain 0.12.3` (LICENSE):

```
Copyright (c) 2018-2025, NLnet Labs. All rights reserved.

This software is open source.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

Redistributions of source code must retain the above copyright notice,
this list of conditions and the following disclaimer.

Redistributions in binary form must reproduce the above copyright notice,
this list of conditions and the following disclaimer in the documentation
and/or other materials provided with the distribution.

Neither the name of the copyright holder nor the names of its contributors
may be used to endorse or promote products derived from this software
without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED
TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### Zlib

As shipped by `bytemuck 1.25.2` (LICENSE-ZLIB):

```
Copyright (c) 2019 Daniel "Lokathor" Gee.

This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

### Unicode-3.0

As shipped by `unicode-ident 1.0.27` (LICENSE-UNICODE):

```
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```

### BSL-1.0

As shipped by `ryu 1.0.23` (LICENSE-BOOST):

```
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### ISC, 0BSD, Unlicense

Short standard texts that ship inside the crates themselves (`isc`/`0bsd`/
`unlicense` directories under `~/.cargo/registry/src/`) and are not reproduced
here:

- **ISC**: riscv, riscv-pac, riscv-rt, riscv-target-parser - canonical text: https://spdx.org/licenses/ISC.html
- **0BSD**: managed, smoltcp, xarxa-driver - canonical text: https://spdx.org/licenses/0BSD.html
- **Unlicense**: aho-corasick, byteorder, jiff, jiff-core, jiff-static, memchr, termcolor, winapi-util - canonical text: https://spdx.org/licenses/Unlicense.html

## Obligations when you ship an image

1. Include this file (or an equivalent notice) with the image or the source you
   hand over.
2. MIT/ISC/BSD/0BSD/Unlicense/Zlib/BSL-1.0 require the copyright notice and the
   permission text to travel with the binary; the per-crate copyright lines live in
   each crate (not reproduced per-package here).
3. Apache-2.0 requires a copy of the license and, if the work ships a `NOTICE`
   file, that its contents are preserved.
4. For a release artifact, generate a fully bundled attribution set instead of this
   summary - `cargo-about` or `cargo-deny` produce one, with the texts and
   per-package copyright lines.

## What would change the picture

- **LGPL** + Rust's static linking: the relink obligation (provide the object files
  or a way to relink) applies, which is awkward in a firmware image.
- **AGPL**: the device serves HTTP on the LAN, which is exactly the use that AGPL
  section 13 targets - the source obligation would trigger on *network use*, not on
  distribution.
- **GPL-3 in a shipped device**: section 6's installation-information requirement
  means you cannot lock the boot path (Secure Boot / flash encryption) without
  giving the user a way to install a modified firmware. Relevant if this reader ends
  up inside a shipped machine.
- **MPL/CDDL/EPL**: file-level copyleft, fine with static linking as long as you
  publish modifications to *those* files.

## This project's own license

MIT OR Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The
audit above is what makes that choice available: with no copyleft dependency in the
graph, any license would have been, including a proprietary one.

---

*Generated inventory, not legal advice: it reflects the metadata the crates declare
and where their license files were found. Verify against the crates' own files
before relying on it in a contract.*
