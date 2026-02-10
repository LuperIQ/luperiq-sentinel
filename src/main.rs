mod agent;
mod config;
mod llm;
mod messaging;
mod net;
mod platform;
mod security;
mod skills;

#[cfg(feature = "tls")]
mod app;

#[cfg(feature = "tls")]
fn main() {
    app::run();
}

#[cfg(not(feature = "tls"))]
fn main() {
    // The LuperIQ OS userspace binary is a separate crate at:
    //   luperiq-agent-os/kernel/user/sentinel/
    // It uses luperiq-rt and kernel syscalls directly (no_std).
    // This std-based crate cannot cross-compile to the kernel target.
    eprintln!("sentinel: built without TLS (no-std stub)");
    eprintln!("sentinel: the LuperIQ OS binary is at kernel/user/sentinel/");
    eprintln!("sentinel: build with: make -C kernel user-sentinel");
}
