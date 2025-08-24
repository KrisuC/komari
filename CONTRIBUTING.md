# Contributing
Thank you for your interest in contributing, I really appreciate it.

## Contributing Code

Feel free to open a Pull Request if you have a feature you'd like to add to this bot. Whether it's a small fix or a major improvement, contributions of all sizes are welcome.

## Questions & Discussions

If you have any questions about the codebase or want to discuss an idea before submitting a PR, please use the **Discussions** tab on GitHub.

## Building the bot

On Windows, to build the bot, you need to have [Dioxus CLI](https://dioxuslabs.com/cli/), OpenCV and LLVM installed. OpenCV binaries need
to also be statically linked.
```powershell
# You can use vcpkg to install OpenCV
vcpkg install opencv4[contrib,nonfree]:x64-windows-static
```

Once all the dependencies are install, you can run the following command in the root directory of the project:
```powershell
# --release can be removed to build debug build
dx build --release --package ui # CPU backend
dx build --release --package ui -- --features backend/gpu # GPU backend
```

Plain `cargo build` is also possile:
```powershell
cargo build
cargo build --features backend/gpu
```
The `backend` code will still run correctly but the `ui` code will appear as blank.

Building on environment such as WSL2 is currently not supported nor tested.
