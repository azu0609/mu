fn main() {
    connectrpc_build::Config::new()
        .files(&["../../proto/mu/v1/mu.proto"])
        .includes(&["../../proto"])
        .generate_json(false)
        .include_file("_connectrpc.rs")
        .compile()
        .unwrap();
}
