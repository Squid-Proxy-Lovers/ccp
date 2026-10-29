// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const ASSETS: [&str; 10] = [
    "index.html",
    "app.js",
    "app.css",
    "theme.js",
    "favicon.svg",
    "fonts/FragmentMono-OFL.txt",
    "fonts/OverusedGrotesk-OFL.txt",
    "fonts/OverusedGroteskRoman-VF.woff2",
    "fonts/fragment-mono-latin-400-normal.woff2",
    "fonts/fragment-mono-latin-ext-400-normal.woff2",
];
fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=CCP_PANEL_DIST");
    for path in [
        "web/src",
        "web/public",
        "web/index.html",
        "web/package.json",
        "web/package-lock.json",
        "web/vite.config.ts",
        "web/tsconfig.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let output = PathBuf::from(
        env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("missing Cargo output directory"))?,
    )
    .join("panel");
    if let Some(input) = env::var_os("CCP_PANEL_DIST") {
        let input = PathBuf::from(input);
        println!("cargo:rerun-if-changed={}", input.display());
        copy_assets(&input, &output)?;
    } else {
        let web = PathBuf::from(
            env::var_os("CARGO_MANIFEST_DIR")
                .ok_or_else(|| io::Error::other("missing manifest directory"))?,
        )
        .join("web");
        if !web.join("node_modules").is_dir() {
            return Err(io::Error::other("frontend dependencies missing: install Node24 and run npm ci --prefix crates/server/web --ignore-scripts before building the server").into());
        }
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let status = Command::new(npm)
            .current_dir(web)
            .args(["run", "build", "--", "--emptyOutDir", "--outDir"])
            .arg(&output)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("frontend build failed").into());
        }
    }
    for asset in ASSETS {
        if !fs::symlink_metadata(output.join(asset))?.is_file() {
            return Err(io::Error::other("frontend asset must be a regular file").into());
        }
    }
    Ok(())
}
fn copy_assets(input: &Path, output: &Path) -> io::Result<()> {
    for asset in ASSETS {
        let source = input.join(asset);
        let destination = output.join(asset);
        if !fs::symlink_metadata(&source)?.is_file() {
            return Err(io::Error::other(
                "prebuilt frontend asset must be a regular file",
            ));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}
