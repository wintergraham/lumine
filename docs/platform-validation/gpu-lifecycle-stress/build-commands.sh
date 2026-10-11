# Official sources; tag resolutions recorded in build-provenance.json.
git clone --depth 1 --branch v1.4.345 https://github.com/KhronosGroup/Vulkan-Loader.git /tmp/lumine-vulkan-loader-1.4.345
git clone --depth 1 --branch v1.4.345 https://github.com/KhronosGroup/Vulkan-Headers.git /tmp/lumine-vulkan-loader-1.4.345/headers
# CMake 3.31.6 official binary archive SHA256 verified against official manifest.
/tmp/cmake-3.31.6-linux-x86_64/bin/cmake -S /tmp/lumine-vulkan-loader-1.4.345/headers -B /tmp/lumine-vulkan-loader-1.4.345/headers-build -DCMAKE_INSTALL_PREFIX=/tmp/lumine-vulkan-loader-1.4.345/headers-install
/tmp/cmake-3.31.6-linux-x86_64/bin/cmake --install /tmp/lumine-vulkan-loader-1.4.345/headers-build
/tmp/cmake-3.31.6-linux-x86_64/bin/cmake -S /tmp/lumine-vulkan-loader-1.4.345 -B /tmp/lumine-vulkan-loader-1.4.345/build -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_PREFIX_PATH=/tmp/lumine-vulkan-loader-1.4.345/headers-install -DCMAKE_INSTALL_PREFIX=/tmp/lumine-vulkan-loader-1.4.345/install -DBUILD_TESTS=OFF
/tmp/cmake-3.31.6-linux-x86_64/bin/cmake --build /tmp/lumine-vulkan-loader-1.4.345/build -j 4
# No loader installation or GPU execution performed by the builder.
# Set LD_LIBRARY_PATH=/tmp/lumine-vulkan-loader-1.4.345/build/loader for the scoped probe.
