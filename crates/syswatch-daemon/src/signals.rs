use std::{
    io,
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
};

use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::{Handle, Signals},
};

pub struct SignalHandle {
    handle: Handle,
    thread: JoinHandle<()>,
}

pub fn install() -> io::Result<(Receiver<i32>, SignalHandle)> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let (tx, rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("syswatch-signals".into())
        .spawn(move || {
            for signal in signals.forever() {
                if tx.send(signal).is_err() {
                    break;
                }
            }
        })?;
    Ok((rx, SignalHandle { handle, thread }))
}

impl SignalHandle {
    pub fn stop(self) {
        self.handle.close();
        self.thread.join().expect("signal thread panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::install;

    #[test]
    fn signal_handlers_install_and_stop_without_leaking_a_thread() {
        let (rx, handle) = install().unwrap();
        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        handle.stop();
    }
}
