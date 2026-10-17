//! Build the Next.js frontend and make sure `frontend/out` exists before
//! `rust-embed` compiles the assets into the binary.
//!
//! Set `BMP_SKIP_FRONTEND=1` to reuse the existing `frontend/out` (or a tiny
//! placeholder if none exists). This is useful on build machines without Node.js.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let frontend = manifest.join("../frontend");
    let out_index = frontend.join("out/index.html");

    for relative in [
        "app",
        "components",
        "lib",
        "public",
        "package.json",
        "package-lock.json",
        "next.config.ts",
        "tsconfig.json",
    ] {
        let path = frontend.join(relative);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed=BMP_SKIP_FRONTEND");

    if env::var("BMP_SKIP_FRONTEND").ok().as_deref() == Some("1") {
        ensure_placeholder(&frontend, &out_index);
        return;
    }

    let rebuild = !out_index.exists() || sources_are_newer(&frontend, &out_index);
    if rebuild {
        build_frontend(&frontend);
    }
    if !out_index.exists() {
        panic!(
            "frontend build did not produce {}. Run `npm install && npm run build` in {} \
             or set BMP_SKIP_FRONTEND=1 to embed a placeholder.",
            out_index.display(),
            frontend.display()
        );
    }
}

fn sources_are_newer(frontend: &Path, out_index: &Path) -> bool {
    let out_mtime = match fs::metadata(out_index).and_then(|m| m.modified()) {
        Ok(time) => time,
        Err(_) => return true,
    };
    let mut newest = SystemTime::UNIX_EPOCH;
    for relative in [
        "app",
        "components",
        "lib",
        "public",
        "package.json",
        "package-lock.json",
        "next.config.ts",
        "tsconfig.json",
    ] {
        newest = newest.max(newest_mtime(&frontend.join(relative)));
    }
    newest > out_mtime
}

fn newest_mtime(path: &Path) -> SystemTime {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return SystemTime::UNIX_EPOCH,
    };
    let mut newest = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if metadata.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name == "node_modules" || name == ".next" || name == "out" {
                    continue;
                }
                newest = newest.max(newest_mtime(&entry.path()));
            }
        }
    }
    newest
}

fn build_frontend(frontend: &Path) {
    if !frontend.exists() {
        panic!("frontend directory {} does not exist", frontend.display());
    }
    if !frontend.join("node_modules").exists() {
        println!("cargo:warning=Installing frontend dependencies (npm install)...");
        run_npm(frontend, &["install", "--no-audit", "--no-fund"]);
    }
    println!("cargo:warning=Building Next.js frontend...");
    run_npm(frontend, &["run", "build"]);
}

fn run_npm(frontend: &Path, args: &[&str]) {
    let mut command = Command::new("npm");
    command
        .args(args)
        .current_dir(frontend)
        .env("NEXT_TELEMETRY_DISABLED", "1");
    let status = command.status().unwrap_or_else(|error| {
        panic!(
            "failed to run `npm {}` in {}: {error}. \
             Install Node.js (>=20) or set BMP_SKIP_FRONTEND=1 to reuse the existing frontend build.",
            args.join(" "),
            frontend.display()
        )
    });
    if !status.success() {
        panic!(
            "`npm {}` failed in {}. Fix the frontend build or set BMP_SKIP_FRONTEND=1.",
            args.join(" "),
            frontend.display()
        );
    }
}

fn ensure_placeholder(frontend: &Path, out_index: &Path) {
    if out_index.exists() {
        return;
    }
    let out_dir = frontend.join("out");
    if let Err(error) = fs::create_dir_all(&out_dir) {
        panic!("failed to create {}: {error}", out_dir.display());
    }
    let html = r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>BMP Status</title>
<style>body{background:#060913;color:#e2e8f0;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0}
.card{max-width:640px;padding:36px 40px;border:1px solid #26324a;border-radius:18px;background:#0b1220}
h1{font-size:20px;margin:0 0 12px}code{background:#131c2e;padding:3px 7px;border-radius:6px;color:#22d3ee}
p{line-height:1.7;color:#93a4bd}a{color:#22d3ee}</style></head><body><div class="card">
<h1>BMP Status 前端未嵌入</h1><p>构建时设置了 <code>BMP_SKIP_FRONTEND=1</code>，且没有现成的 <code>frontend/out</code>。</p>
<p>请执行 <code>cd frontend &amp;&amp; npm install &amp;&amp; npm run build</code> 后重新编译 Rust 二进制。</p>
<p>API：<a href="/api/health">/api/health</a> · <a href="/api/overview">/api/overview</a></p>
</div></body></html>"#;
    if let Err(error) = fs::write(out_index, html) {
        panic!("failed to write {}: {error}", out_index.display());
    }
}
