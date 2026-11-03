use super::*;
struct DropNotice(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for DropNotice {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}
struct Writer {
    held: Arc<Mutex<Vec<Ticket>>>,
    mode: u8,
    writes: usize,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        assert_eq!(
            self.held.lock().unwrap().len(),
            1,
            "ticket must be retained before first I/O"
        );
        self.writes += 1;
        if self.mode == 1 && self.writes > 1 {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        Ok(if self.mode == 1 {
            1.min(bytes.len())
        } else {
            bytes.len()
        })
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.mode == 2 {
            Err(std::io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
}
#[test]
fn successful_and_uncertain_notification_writes_retain_until_scope_cleanup() {
    for mode in [0, 1, 2] {
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticket: Ticket = Arc::new(DropNotice(dropped.clone()));
        let held = Arc::new(Mutex::new(Vec::new()));
        let mut writer = Writer {
            held: held.clone(),
            mode,
            writes: 0,
        };
        let result = write_retaining_operation(
            &mut writer,
            b"notify
",
            Some(ticket),
            &held,
        );
        assert_eq!(result.is_ok(), mode == 0);
        assert_eq!(dropped.load(Ordering::Acquire), 0);
        assert_eq!(held.lock().unwrap().len(), 1);
        held.lock().unwrap().clear();
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }
}
