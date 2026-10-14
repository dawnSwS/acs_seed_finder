use std::env;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rustc_version() -> String {
    let Some(rustc) = env::var_os("RUSTC") else {
        return "rustc_version_status=error\nrustc_version_error=RUSTC is not set\n".into();
    };
    match Command::new(rustc).args(["--version", "--verbose"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let status = if output.status.success() && !stdout.trim().is_empty() {
                "rustc_version_status=ok\n".to_owned()
            } else {
                format!(
                    "rustc_version_status=error\nrustc_version_error=exit status {}; empty stdout={}\n",
                    output.status,
                    stdout.trim().is_empty()
                )
            };
            format!("{status}--- RUSTC --version --verbose stdout ---\n{stdout}\n--- stderr ---\n{stderr}\n")
        }
        Err(error) => format!(
            "rustc_version_status=error\nrustc_version_error=could not execute RUSTC --version --verbose: {error}\n"
        ),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    let mut info = String::from("build_info_schema=1\n");
    for name in ["TARGET", "PROFILE", "OPT_LEVEL", "DEBUG"] {
        println!("cargo:rerun-if-env-changed={name}");
        match env::var(name) {
            Ok(value) => writeln!(info, "{name}={value}")?,
            Err(error) => writeln!(info, "{name}=unknown; environment error: {error}")?,
        }
    }
    println!("cargo:rerun-if-env-changed=RUSTC");
    info.push_str(&rustc_version());
    info.push_str(
        "wgpu_resolved_version=unknown\n\
         dependency_version_note=No resolved dependency graph is supplied to this build script. \
         No Cargo.lock is read or assumed. Research CI cargo metadata/tree records can establish \
         the exact resolved dependency versions for that CI build.\n",
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    fs::write(out_dir.join("build-info.txt"), info)?;
    Ok(())
}
