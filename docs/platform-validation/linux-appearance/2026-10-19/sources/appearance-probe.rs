#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAppearance { Pending, Unsupported, Light, Dark }
#[path="../crates/lumine-desktop/src/appearance_linux.rs"]
mod appearance_linux;
fn main() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut appearance = appearance_linux::Appearance::new(std::sync::Arc::new(move |event| { let _ = sender.send(event); }));
    let mut state = appearance.state();
    println!("appearance initial={state:?}");
    let start=std::time::Instant::now();
    let mut events=0;
    while let Some(remaining) = std::time::Duration::from_secs(15).checked_sub(start.elapsed()) {
        match receiver.recv_timeout(remaining) {
            Ok(event) => {
                events+=1;
                let next=appearance.handle(event);
                if next!=state { println!("appearance event={event:?} state={next:?} elapsed={:?}",start.elapsed()); state=next; }
            },
            Err(_) => break,
        }
    }
    println!("appearance events={events} final={state:?}");
}
