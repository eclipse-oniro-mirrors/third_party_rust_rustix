use std::process::Command;

#[test]
fn test_backends() {
    // Test whether `has_dependency` works. `cargo tree` no longer works in
    // Rust 1.48 because `cargo tree` pulls in dependencies for all targets,
    // and hermit-core isn't compatible with Rust 1.48.
    if !has_dependency(".", &[], &[], &[], "io-lifetimes") {
        return;
    }

    // Pick an arbitrary platform where linux_raw is enabled by default and
    // ensure that the use-default crate uses it.
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        assert!(
            has_dependency(
                "test-crates/use-default",
                &[],
                &[],
                &["RUSTFLAGS"],
                "linux-raw-sys"
            ),
            "use-default does not depend on linux-raw-sys"
        );
    }

    // Pick an arbitrary platform where linux_raw is enabled by default and
    // ensure that the use-rustix-auxv crate uses it, and does not use libc.
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        // TODO: Re-enable this test once io-lifetimes can depend on Rust 1.63
        // and always use std, so it can drop its libc dependency.
        /*
        assert!(
            !has_dependency(
                "test-crates/use-rustix-auxv",
                &[],
                &[],
                &["RUSTFLAGS"],
                "libc"
            ),
            "use-rustix-auxv depends on libc"
        );
        */

        assert!(
            has_dependency(
                "test-crates/use-rustix-auxv",
                &[],
                &[],
                &["RUSTFLAGS"],
                "linux-raw-sys"
            ),
            "use-rustix-auxv does not depend on linux-raw-sys"
        );
    }

    #[cfg(windows)]
    let libc_dep = "windows-sys";
    #[cfg(any(unix, target_os = "wasi"))]
    let libc_dep = "libc";

    // FIXME: Temporarily disable the subsequent tests on Windows until
    // rust-errno updates to windows-sys 0.48.
    #[cfg(windows)]
    {
        if true {
            return;
        }
    }

    // Test the use-libc crate, which enables the "use-libc" cargo feature.
    assert!(
        has_dependency("test-crates/use-libc", &[], &[], &["RUSTFLAGS"], libc_dep),
        "use-libc doesn't depend on {}",
        libc_dep
    );

    // Test the use-default crate with `--cfg=rustix_use_libc`.
    assert!(
        has_dependency(
            "test-crates/use-default",
            &[],
            &[("RUSTFLAGS", "--cfg=rustix_use_libc")],
            &[],
            libc_dep
        ),
        "use-default with --cfg=rustix_use_libc does not depend on {}",
        libc_dep
    );

    // Test the use-default crate with `--features=rustix/use-libc`.
    assert!(
        has_dependency(
            "test-crates/use-default",
            &["--features=rustix/use-libc"],
            &[],
            &[],
            libc_dep
        ),
        "use-default with --features=rustix/use-libc does not depend on {}",
        libc_dep
    );
}

/// Test whether the crate at directory `dir` has a dependency on `dependency`,
/// setting the environment variables `envs` and unsetting the environment
/// variables `remove_envs` when running `cargo`.
fn has_dependency(
    dir: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    remove_envs: &[&str],
    dependency: &str,
) -> bool {
    let mut command = Command::new("cargo");

    command
        .arg("tree")
        .arg("--quiet")
        .arg("--edges=normal")
        .arg(&format!("--invert={}", dependency))
        .current_dir(dir);

    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    for key in remove_envs {
        command.env_remove(key);
    }

    let child = command.output().unwrap();

    // `cargo tree --invert=foo` can fail in two different ways: it exits with
    // a non-zero status if the dependency is not present in the Cargo.toml
    // configuration, and it exists with a zero status and prints nothing if
    // the dependency is present but optional and not enabled. So we check for
    // both here.
    child.status.success() && !child.stdout.is_empty()
}
