#!/usr/bin/env zsh
set -euo pipefail
sdk=${1:?Pass the pinned VST3 SDK checkout}
sdk_build=${2:?Pass its completed Release library build}
destination=${3:?Pass a project-owned fixture output directory}
expected=3cdf9ca5d1f5b1b21e0a86832aa4abe55607bd96
[[ $(git -C "$sdk" rev-parse HEAD) == "$expected" ]] || { print -u2 'VST3 SDK revision differs from the qualified fixture'; exit 1; }
arch=$(uname -m)
[[ $arch == aarch64 || $arch == x86_64 ]] || { print -u2 'Unsupported native fixture architecture'; exit 1; }
mkdir -p "$destination/Contract.vst3/Contents/$arch-linux"
source_dir=${0:A:h}
g++ -std=c++17 -O2 -DNDEBUG -DRELEASE -fPIC -shared -I"$sdk" "$source_dir/contract.cpp" "$sdk/public.sdk/source/main/linuxmain.cpp" -Wl,--start-group "$sdk_build/lib/Release/libsdk.a" "$sdk_build/lib/Release/libsdk_common.a" "$sdk_build/lib/Release/libbase.a" "$sdk_build/lib/Release/libpluginterfaces.a" -Wl,--end-group -lpthread -ldl -o "$destination/Contract.vst3/Contents/$arch-linux/Contract.so"
