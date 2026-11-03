#[cfg(target_os = "linux")]
fn main() {
    match gent_linux_cgroup_owner::prepare_current_service() {
        Ok(()) => println!("own-service-memory-pids-prepared"),
        Err(error) => {
            eprintln!("service preparation rejected: {error:?}");
            std::process::exit(1);
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    std::process::exit(78);
}
