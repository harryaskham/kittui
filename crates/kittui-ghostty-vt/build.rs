fn main() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_NO_PKG_CONFIG");
    if std::env::var_os("LIBGHOSTTY_VT_NO_PKG_CONFIG").is_some() {
        return;
    }
    pkg_config::Config::new()
        .atleast_version("0.1")
        .probe("libghostty-vt")
        .expect("libghostty-vt pkg-config metadata not found; enter `nix develop` or install nixpkgs#libghostty-vt.dev");
}
