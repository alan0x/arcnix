use std::path::PathBuf;
use std::{env, fs, io};

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && env::var_os("DEP_OPENSSL_VENDORED").is_some()
    {
        let install = PathBuf::from(
            env::var_os("DEP_OPENSSL_ROOT").expect("vendored OpenSSL install directory"),
        );
        let pdb = install.join("lib/ossl_static.pdb");
        println!("cargo::rerun-if-changed={}", pdb.display());

        // openssl-src installs the PDB, then removes the build/src directory whose
        // absolute path MSVC recorded in the objects bundled by libssh2-sys.
        let object_dir = install.with_file_name("build").join("src");
        fs::create_dir_all(&object_dir)?;
        fs::copy(pdb, object_dir.join("ossl_static.pdb"))?;
    }
    Ok(())
}
