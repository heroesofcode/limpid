# Changelog

## [0.4.0](https://github.com/heroesofcode/limpid/compare/v0.3.0...v0.4.0) (2026-10-03)


### Features

* a config file, and exclusions that are never offered or removed ([#24](https://github.com/heroesofcode/limpid/issues/24)) ([a76939b](https://github.com/heroesofcode/limpid/commit/a76939b49ee78d3af6060e62dd99a1a37f53d51f))
* exclude from the overview and the storage view, and a settings page ([#25](https://github.com/heroesofcode/limpid/issues/25)) ([ab5ce16](https://github.com/heroesofcode/limpid/commit/ab5ce1671950acbc1743980929d52730a43c99d3))
* find build output in projects, by a rule the guard checks again ([#27](https://github.com/heroesofcode/limpid/issues/27)) ([3344d2f](https://github.com/heroesofcode/limpid/commit/3344d2fd62a00b10a9243cba46f9e321fd3d18bd))
* say what is ready, list exactly what goes, and question large plans ([4ce82b0](https://github.com/heroesofcode/limpid/commit/4ce82b027b0546c1d3bd6215552e97f3174ca5eb))
* tick folders in the storage view, and send them to the trash whole ([689763b](https://github.com/heroesofcode/limpid/commit/689763ba4398805a36b9867a5d1da006b81b4008))


### Bug fixes

* make the release tarball installable, and correct what the roadmap said ([#22](https://github.com/heroesofcode/limpid/issues/22)) ([847f4f1](https://github.com/heroesofcode/limpid/commit/847f4f18f3d439a86229f8c872dc424a1c4f7472))
* refuse a theme colour rather than panic on a multi-byte character ([541b7f5](https://github.com/heroesofcode/limpid/commit/541b7f5529edb6544dd37a5de391a8e38d05a246))


### Refactoring

* replace Makefile to Mise ([#32](https://github.com/heroesofcode/limpid/issues/32)) ([eab9cf2](https://github.com/heroesofcode/limpid/commit/eab9cf2dfbee9760182cb841026463ea7e16e1be))

## [0.3.0](https://github.com/heroesofcode/limpid/compare/v0.2.1...v0.3.0) (2026-09-28)


### Features

* act on files from the storage view ([04dfad5](https://github.com/heroesofcode/limpid/commit/04dfad527af7e85de12f923cb0100237573aba53))
* give the guard a second permission, for what the user points at ([#19](https://github.com/heroesofcode/limpid/issues/19)) ([9ea9b20](https://github.com/heroesofcode/limpid/commit/9ea9b202f3e55a781b71c4dbe41eaf76df866f63))
* lay the interface out for the room it actually has ([#14](https://github.com/heroesofcode/limpid/issues/14)) ([c4fe746](https://github.com/heroesofcode/limpid/commit/c4fe7468953b9e4299b7314a0299a0865d539bc2))
* reveal a file in the file manager, and copy its path ([#21](https://github.com/heroesofcode/limpid/issues/21)) ([b5c776f](https://github.com/heroesofcode/limpid/commit/b5c776f775166afcbed97513b59c0c124f4d60b5))


### Bug fixes

* close the four holes that acting from the storage view would widen ([9b972cc](https://github.com/heroesofcode/limpid/commit/9b972ccd39de2c9f0c6754314e359998ae33faf7))

## [0.2.1](https://github.com/heroesofcode/limpid/compare/v0.2.0...v0.2.1) (2026-09-26)


### Bug fixes

* build the release binaries in the run that creates the release ([#12](https://github.com/heroesofcode/limpid/issues/12)) ([a5faf76](https://github.com/heroesofcode/limpid/commit/a5faf767e822d9911bf833f3494cefb5d5b52bb3))

## [0.2.0](https://github.com/heroesofcode/limpid/compare/v0.1.0...v0.2.0) (2026-09-26)


### Features

* add the application window ([#3](https://github.com/heroesofcode/limpid/issues/3)) ([0deae5e](https://github.com/heroesofcode/limpid/commit/0deae5e1f55d7bbfd2e06a125d76f75a10bd13dc))
* do the parts that need root, without handing root a path ([#7](https://github.com/heroesofcode/limpid/issues/7)) ([a1fff75](https://github.com/heroesofcode/limpid/commit/a1fff75e890e70ff57db061c7adf9bc0e1644e96))
* find and clean browser caches, when the browser is closed ([#5](https://github.com/heroesofcode/limpid/issues/5)) ([76de438](https://github.com/heroesofcode/limpid/commit/76de438b4a4d9041cae133c16de27921f773c0c0))
* follow the Omarchy theme, live ([#2](https://github.com/heroesofcode/limpid/issues/2)) ([8a240b9](https://github.com/heroesofcode/limpid/commit/8a240b9627993a03d08c4fb7c4c310fb8c3cc646))
* make it installable ([7b1527d](https://github.com/heroesofcode/limpid/commit/7b1527d2cbee698ccc844f08a9477226e9b5ed30))
* measure reclaimable space without touching anything ([#1](https://github.com/heroesofcode/limpid/issues/1)) ([a40e6b9](https://github.com/heroesofcode/limpid/commit/a40e6b9ce1ed52db3b3f1a2d14a32a5796058641))
* remove what a scan found, carefully ([#4](https://github.com/heroesofcode/limpid/issues/4)) ([690ce6c](https://github.com/heroesofcode/limpid/commit/690ce6c4a3b184472d4e141050bd53049ad26cff))
* show where the space went ([#6](https://github.com/heroesofcode/limpid/issues/6)) ([a4cbbe7](https://github.com/heroesofcode/limpid/commit/a4cbbe7a80f3f64ec3f7603dcef7ccb269ef46c5))


### Bug fixes

* drop the version requirement from the internal path dependencies ([#11](https://github.com/heroesofcode/limpid/issues/11)) ([a9fdef4](https://github.com/heroesofcode/limpid/commit/a9fdef4b1719d12cc3030532d2e145fc6a96df75))
* give release-please a package manifest to work from ([#9](https://github.com/heroesofcode/limpid/issues/9)) ([f1a24f0](https://github.com/heroesofcode/limpid/commit/f1a24f0a1eeb5655a5ec03330ac9e371d5ca03ca))
* let release-please see the workspace versions ([33b4285](https://github.com/heroesofcode/limpid/commit/33b428537bd26b4ac9294422906ad577d31d3d05))
