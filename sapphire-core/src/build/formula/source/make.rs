// sapphire-core/src/build/formula/source/make.rs

use std::fs;
use std::io::Read; // <--- Add Read trait for reading file content
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tracing::{debug, error, info, warn};

use crate::build::env::BuildEnvironment;
use crate::utils::error::{Result, SapphireError};

/// Checks if a configure script appears to be generated by GNU Autotools.
fn is_gnu_autotools_configure(script_path: &Path) -> bool {
    const READ_BUFFER_SIZE: usize = 4096; // Read first 4KB
    const AUTOCONF_MARKERS: &[&str] = &[
        "Generated by GNU Autoconf", // Common marker
        "generated by autoconf",     // Another possible marker
        "config.status:",            // Often present in generated scripts
    ];

    let mut buffer = String::with_capacity(READ_BUFFER_SIZE);
    match fs::File::open(script_path)
        .and_then(|mut file| file.read_to_string(&mut buffer)) // Read directly into String buffer
    {
        Ok(_) => {
            // Check if any marker is present in the read portion
            for marker in AUTOCONF_MARKERS {
                if buffer.contains(marker) {
                    debug!(
                        "Detected Autotools marker ('{}') in configure script: {}",
                        marker,
                        script_path.display()
                    );
                    return true; // Found a marker
                }
            }
            debug!(
                "No specific Autotools markers found in first {} bytes of configure script: {}",
                READ_BUFFER_SIZE,
                script_path.display()
            );
            false // No markers found
        }
        Err(e) => {
            warn!(
                "Could not read configure script {} to check for Autotools markers: {}. Assuming not Autotools.",
                script_path.display(), e
            );
            false // Failed to read, assume not Autotools
        }
    }
}

/// Configure and build with potentially Autotools script (./configure && make && make install)
pub fn configure_and_make(install_dir: &Path, build_env: &BuildEnvironment) -> Result<()> {
    let configure_script_path = Path::new("./configure"); // Assuming CWD is build_dir

    // Check if configure script exists before trying to detect/run
    if !configure_script_path.exists() {
        tracing::error!("./configure script not found in current directory.");
        // This case should ideally be caught by detect_and_build, but check defensively.
        return Err(SapphireError::BuildEnvError(
            "configure script not found, cannot run Autotools build.".to_string(),
        ));
    }

    // *** Detect if it's likely an Autotools script ***
    let is_autotools = is_gnu_autotools_configure(configure_script_path);

    info!("==> Running ./configure --prefix={}", install_dir.display());
    if is_autotools {
        info!("    (Detected Autotools, adding standard flags)");
    } else {
        info!("    (Did not detect standard Autotools markers, running configure without Autotools flags)");
    }

    let mut cmd = Command::new(configure_script_path); // Use the PathBuf
    cmd.arg(format!("--prefix={}", install_dir.display()));

    // *** Conditionally add Autotools flags ***
    if is_autotools {
        cmd.args(["--disable-dependency-tracking", "--disable-silent-rules"]);
    }

    build_env.apply_to_command(&mut cmd);
    let output = cmd.output().map_err(|e| {
        SapphireError::CommandExecError(format!("Failed to execute configure: {}", e))
    })?;

    if !output.status.success() {
        println!("Configure failed with status: {}", output.status);
        eprintln!(
            "Configure stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        eprintln!(
            "Configure stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config_log_path = std::path::PathBuf::from("config.log");
        if config_log_path.exists() {
            eprintln!("--- Last 50 lines of config.log ---");
            if let Ok(content) = fs::read_to_string(&config_log_path) {
                let lines: Vec<&str> = content.lines().rev().take(50).collect();
                for line in lines.iter().rev() {
                    eprintln!("{}", line);
                }
            }
            eprintln!("--- End config.log ---");
        }
        return Err(SapphireError::Generic(format!(
            "Configure failed with status: {}",
            output.status
        )));
    } else {
        debug!(
            "Configure stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        debug!(
            "Configure stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // --- make && make install steps remain the same ---
    info!("==> Running make");
    let make_exe = which::which_in("make", build_env.get_path_string(), Path::new("."))
        .or_else(|_| which::which("make"))
        .map_err(|_| {
            SapphireError::BuildEnvError(
                "make command not found in build environment PATH or system PATH.".to_string(),
            )
        })?;
    let mut cmd_make = Command::new(make_exe.clone());
    build_env.apply_to_command(&mut cmd_make);
    let output_make = cmd_make
        .output()
        .map_err(|e| SapphireError::CommandExecError(format!("Failed to execute make: {}", e)))?;

    if !output_make.status.success() {
        println!("Make failed with status: {}", output_make.status);
        eprintln!(
            "Make stdout:\n{}",
            String::from_utf8_lossy(&output_make.stdout)
        );
        eprintln!(
            "Make stderr:\n{}",
            String::from_utf8_lossy(&output_make.stderr)
        );
        return Err(SapphireError::Generic(format!(
            "Make failed with status: {}",
            output_make.status
        )));
    } else {
        debug!("Make completed successfully.");
    }

    info!("==> Running make install");
    let mut cmd_install = Command::new(make_exe);
    cmd_install.arg("install");
    build_env.apply_to_command(&mut cmd_install);
    let output_install = cmd_install.output().map_err(|e| {
        SapphireError::CommandExecError(format!("Failed to execute make install: {}", e))
    })?;

    if !output_install.status.success() {
        println!("Make install failed with status: {}", output_install.status);
        eprintln!(
            "Make install stdout:\n{}",
            String::from_utf8_lossy(&output_install.stdout)
        );
        eprintln!(
            "Make install stderr:\n{}",
            String::from_utf8_lossy(&output_install.stderr)
        );
        return Err(SapphireError::Generic(format!(
            "Make install failed with status: {}",
            output_install.status
        )));
    } else {
        debug!("Make install completed successfully.");
    }

    Ok(())
}

pub fn simple_make(
    install_dir: &Path, // e.g., /opt/homebrew/Cellar/doggo/1.0.5
    build_env: &BuildEnvironment,
) -> Result<()> {
    info!("==> Building with simple Makefile");
    let make_exe = which::which_in("make", build_env.get_path_string(), Path::new("."))
        .or_else(|_| which::which("make")) // Fallback
        .map_err(|_| {
            SapphireError::BuildEnvError(
                "make command not found in build environment PATH or system PATH.".to_string(),
            )
        })?;

    info!("==> Running make");
    let mut cmd_make = Command::new(make_exe.clone());
    build_env.apply_to_command(&mut cmd_make);
    // Assuming CWD is the build directory (e.g., ./doggo-1.0.5/)
    // Let's capture the output for potential debugging if needed
    let output_make = cmd_make.output().map_err(|e| {
        SapphireError::CommandExecError(format!("Failed to execute make (simple): {}", e))
    })?;

    if !output_make.status.success() {
        println!("Make failed with status: {}", output_make.status);
        eprintln!(
            "Make stdout:\n{}",
            String::from_utf8_lossy(&output_make.stdout)
        );
        eprintln!(
            "Make stderr:\n{}",
            String::from_utf8_lossy(&output_make.stderr)
        );
        return Err(SapphireError::Generic(format!(
            "Make failed with status: {}",
            output_make.status
        )));
    } else {
        info!("Make completed successfully.");
        // Optionally log build output if verbose enough
        debug!(
            "Make stdout:\n{}",
            String::from_utf8_lossy(&output_make.stdout)
        );
        debug!(
            "Make stderr:\n{}",
            String::from_utf8_lossy(&output_make.stderr)
        );
    }

    // --- Attempt make install ---
    info!("==> Running make install PREFIX={}", install_dir.display());
    let mut cmd_install = Command::new(make_exe);
    cmd_install.arg("install");
    // Pass PREFIX, but be prepared for it to be ignored or incomplete
    cmd_install.arg(format!("PREFIX={}", install_dir.display()));
    build_env.apply_to_command(&mut cmd_install);
    let output_install = cmd_install.output().map_err(|e| {
        SapphireError::CommandExecError(format!("Failed to execute make install (simple): {}", e))
    })?;

    let make_install_succeeded = output_install.status.success();

    if !make_install_succeeded {
        // Log the failure but don't necessarily error out yet
        warn!(
            "'make install' failed with status {}. Will check for manually installable artifacts.",
            output_install.status
        );
        debug!(
            "Make install stdout:\n{}",
            String::from_utf8_lossy(&output_install.stdout)
        );
        debug!(
            "Make install stderr:\n{}",
            String::from_utf8_lossy(&output_install.stderr)
        );
    } else {
        info!("Make install completed successfully (exit code 0).");
        debug!(
            "Make install stdout:\n{}",
            String::from_utf8_lossy(&output_install.stdout)
        );
        debug!(
            "Make install stderr:\n{}",
            String::from_utf8_lossy(&output_install.stderr)
        );
    }

    // --- Verification and Manual Installation Fallback ---
    let bin_dir = install_dir.join("bin");
    let bin_populated = bin_dir.is_dir() && bin_dir.read_dir()?.next().is_some();

    if !bin_populated {
        warn!(
            "Installation directory '{}' is empty or missing after 'make install'. Attempting manual artifact installation.",
            bin_dir.display()
        );

        // Try to find the executable in the CWD (build dir, e.g., ./doggo-1.0.5/)
        // Heuristic: look for a file named like the install dir's base name (e.g., "doggo")
        let formula_name = install_dir
            .parent() // Get .../Cellar/doggo
            .and_then(|p| p.file_name()) // Get "doggo"
            .and_then(|n| n.to_str())
            .unwrap_or(""); // Fallback to empty string if path parsing fails

        let potential_binary_path = Path::new(".").join(formula_name); // Assumes CWD is build root
        let mut found_and_installed_manually = false;

        if !formula_name.is_empty() && potential_binary_path.is_file() {
            info!(
                "Found potential binary '{}' in build directory. Manually installing...",
                potential_binary_path.display()
            );
            fs::create_dir_all(&bin_dir)?; // Ensure install_dir/bin exists

            let target_path = bin_dir.join(formula_name);
            fs::copy(&potential_binary_path, &target_path).map_err(|e| {
                SapphireError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to copy binary {} to {}: {}",
                        potential_binary_path.display(),
                        target_path.display(),
                        e
                    ),
                ))
            })?;

            // Set executable permissions
            #[cfg(unix)]
            {
                let mut perms = fs::metadata(&target_path)?.permissions();
                perms.set_mode(0o755); // rwxr-xr-x
                fs::set_permissions(&target_path, perms)?;
                info!("Set executable permissions on {}", target_path.display());
            }

            found_and_installed_manually = true;
        } else {
            // Optional: Could add more heuristics here, like searching for any executable file
            // in the CWD if the named one isn't found.
            warn!(
                "Could not find executable named '{}' in build directory for manual installation.",
                formula_name
            );
        }

        // If make install failed AND we couldn't manually install anything, then it's a real error
        if !make_install_succeeded && !found_and_installed_manually {
            error!(
                "make install failed and could not find/install artifacts manually from build directory."
            );
            // Return the original make install error context if available
            return Err(SapphireError::Generic(format!(
                "Make install failed with status: {} and no artifacts found/installed manually",
                output_install.status
            )));
        } else if !found_and_installed_manually {
            // make install succeeded but didn't populate bin, and we found nothing manually.
            // This is suspicious, but maybe the formula only installs libraries or other things.
            // Proceed, but maybe log a higher warning?
            warn!("make install reported success, but '{}' was not populated and no executable found manually.", bin_dir.display());
        }
    } else {
        info!(
            "Installation directory '{}' appears populated after 'make install'.",
            bin_dir.display()
        );
    }

    Ok(())
}
