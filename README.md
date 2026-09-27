# What is it?

This project currently stands as more of a proof of concept that it can
run an aribitrary compute shader on a camera frame with zero copy than
an actual SLAM/VIO,which I plan to implment eventually in the future.

# Demo Video
https://github.com/user-attachments/assets/0d93534c-6414-4732-b3de-6e31f814698d

# How to run

## Linux

``` sh
nix develop
cargo run
```

## Android
``` sh
nix develop .#android
cargo apk run
```

# Why another VIO/SLAM project?
## Little history

### ISRO IRoC 2025

1.  ROVIO

    I was the team head at my institute\'s (IIT Kanpur) Aerial Robotics
    team and I was working on a competition (ISRO Robotics Challenge
    URSC IRoC 2025) which had the requirement creating a GPS denied
    autonomous drone, which wasn\'t a particularly novel thing but
    surprisingly the choices that were available (at least to me) felt
    very limited, there was one which the team has been using called
    \'ROVIO\', now rovio itself was fine but the problem was the
    hardware it was being ran on i.e. Raspberry Pi 4B to be precise, it
    could ran ROVIO but we didn\'t want to just run that we wanted to
    run our visual model (which works fine if ran alone) too on the same
    system but that would have overloaded the system and make the drone
    unstable.

    So, as you could understand that ROVIO wasn\'t cutting it for my
    needs and there was no reason to believe something like ORB-SLAM,
    VINS, OpenVSLAM would as they are even heavier and CPU based I
    wasn\'t having the problem if it being not accurate enough but the
    problem that it didn\'t gave me enough headroom.

2.  VSLAM (on Nvidia Jetson Orin Nano)

    VSLAM which is available on Nvidia Jetson was one good solution it
    gave us enough headroom to run both our visual model and the SLAM
    simultaneously without much issues. One can certainly think that the
    hardware buff is what made this possible and that may be true but
    that doesn\'t mean that was the only reason, VSLAM performed far
    better than any SLAM solution we tried on Jetson.

3.  The Vendor problem

    Using VLSAM tied our team to Nvidia Jetson and that did made things
    a little complicated for the team.Since, we required a very specific
    hardware to run on it it means it would only run on a specific
    version of Nvidia Jetson which if got discontinued means we would
    have to rebuild our pipeline.

### Enter Android

I thought for a while is there a small compute platform which is more
powerful than a Raspberry PI and is readily available, I looked at my
android device and thought this too has a good gpu as I can actually
play games with good graphics on it then there must be some way to run
compute on it.


# Architecture at a glance

## Why Vulkan?

Upon searching I could find that vulkan was the only compute platform on
android that I can reliably expect. Although a lot of Android devices
supports OpenCL the support generally comes from the Chip manufacturer
and was not a standard Android API.

Another benefit of using Vulkan was that it is one of the most widely
available API across all platforms and it specifically supports Compute
together with Graphics as a first class citizen.

## The camera: two very different zero-copy paths

### Linux: V4L2 -\> DMA-BUF -\> vulkano::sys::RawImage

`src/utils/camera/linux.rs` requests 4 mmap buffers from V4L2, then
instead of reading them on the CPU, exports each buffer\'s DMA-BUF file
descriptor (`VIDIOC_EXPBUF`) and imports that fd directly as a Vulkan
image via `RawImage` + `ImportMemoryFdInfo`. Per frame it\'s just:
dequeue a V4L2 buffer, dispatch the `yuvy_to_r8` shader against the
imported image, queue the buffer back. The kernel\'s camera driver and
the GPU share the same physical pages.

### Android: Camera2 -\> AHardwareBuffer -\> immutable Ycbcr sampler

Android has no DMA-BUF fd handed to userspace this way; instead Camera2
hands back an `AHardwareBuffer`, imported via
`VK_ANDROID_external_memory_android_hardware_buffer`. The catch: the
buffer\'s actual pixel layout is vendor-defined, so Vulkan reports
`format=UNDEFINED` and the image can **only** be read through a combined
image sampler with an immutable `VkSamplerYcbcrConversion` attached,
never as a storage image. That forces a second, Android-only shader,
`android_yuvy_to_r8.slang`, which samples `.g` (not `.r`) for
luminance, because the Ycbcr conversion spec fixes G=Y regardless of the
color model chosen. Getting this wrong (as the shader\'s own comment
warns) is the kind of bug that silently gives you chroma instead of
luma.

Both paths converge on the same output: a persistent
`R8_UNORM` luminance image, full camera resolution, that
`FastPass` and `OrbPass` are built against once at startup.

# Feature pipeline (which are implemented for now)

I tried to just run the following shaders to test if the shader pipeline
was working correctly.

1. FAST-9 corner detection
2. ORB orientation
3. Gaussian pyramid

# Cross-compiling with Nix

`flake.nix` builds three targets from one source tree with
[crane](https://github.com/ipetkov/crane) +
[fenix](https://github.com/nix-community/fenix):

- native x86~64~ Linux (`photon-slam`)
- arm64 Linux (`arm64-photon-slam`, cross toolchain)
- Android arm64 via `androidenv`, SDK `min_sdk_version = 33` pinned to
  `target_sdk_version = 30` because the flake\'s bundled SDK only ships
  `android-30`\'s `aapt~/~android.jar`  documented directly in
  `Cargo.toml` rather than left as a mystery version pin.

The payoff of doing this in Nix instead of a shell script: the same
`slangc` + Vulkan SDK + Android NDK versions are pinned for every
contributor and CI, so \"works on my machine\" doesn\'t apply to shader
compilation drift.

# Special Thanks

## Nix

Nix really helped me maintain my sanity by setting up a declarative
android environment.

## Slang

I chosed slang as the shader language for this project as it gives me a
more modern feel and I can reuse the shaders if I want to use this
shader in some other project too.

## Tsoding [Learning Vulkan with Rust Video](https://www.youtube.com/watch?v=8iEN64bj3X4)

This might make even him chuckle a bit if he reads this, but starting
with vulkan was very content heavy, as in you need a lot of prerequisite
knowledge to understand what\'s even going on.His video actually gave me
way to how to actually tackle it in a way so that I could do what I want
without actually getting carried away by the information overload.

## Vulkano

This is an amazing rust library that took out most of the boilerplate for
vulkan.
