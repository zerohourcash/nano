fn main() {
    println!("cargo:rerun-if-env-changed=EVERYDAY_BUILD_REVISION");
}
