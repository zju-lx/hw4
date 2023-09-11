use std::task::{Wake, Waker, RawWaker, RawWakerVTable, Context, Poll::Ready};
use std::future::{Future};
use std::sync::{Arc, Mutex, Condvar};
use std::cell::{RefCell};
use std::collections::{VecDeque};
use std::thread::JoinHandle;
use std::time::Duration;
#[macro_use]
extern crate scoped_tls;
use futures::future::BoxFuture;

const VTABLE: RawWakerVTable = 
    RawWakerVTable::new(vtable_clone, vtable_wake, vtable_wake_by_ref, vtable_drop);

unsafe fn vtable_clone(_p: *const ()) -> RawWaker {
    RawWaker::new(_p, &VTABLE)
}

unsafe fn vtable_wake(_p: *const ()) {}

unsafe fn vtable_wake_by_ref(_p: *const ()) {}

unsafe fn vtable_drop(_p: *const ()) {}

fn dummmy_waker() -> Waker {
    static Data: () = ();
    unsafe { Waker::from_raw(RawWaker::new(&Data, &VTABLE)) }
}

//===================================================================================
struct Signal {
    state: Mutex<State>,
    cond: Condvar,
}

enum State {
    Empty,
    Waiting,
    Notified,
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

impl Signal {

fn new() -> Self {
    Self { state: Mutex::new(State::Notified), cond: Condvar::new() }
}
fn wait(&self) {
    let mut state = self.state.lock().unwrap();
    match *state {
        State::Notified => *state = State::Empty,
        State::Waiting => {
            panic!("multiple wait");
        }
        State::Empty => {
            *state = State::Waiting;
            //while let State::Waiting = *state {
                state = self.cond.wait(state).unwrap();
            //}
        }
    }
}

fn notify(&self) {
    let mut state = self.state.lock().unwrap();
    match *state {
        State::Notified => {}
        State::Empty => *state = State::Notified,
        State::Waiting => {
            *state = State::Empty;
            self.cond.notify_one();
        }
    }
}
}

//=========================================================================================

struct Task {
    future: RefCell<BoxFuture<'static, ()>>,
    signal: Arc<Signal>,
}
scoped_thread_local!(static SIGNAL: Arc<Signal>);
scoped_thread_local!(static RUNNABLE: Mutex<VecDeque<Arc<Task>>>);

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Wake for Task {
fn wake(self: Arc<Self>) {
    RUNNABLE.with(|runnable| {
        runnable.lock().unwrap().push_back(self.clone());
        self.signal.notify();
    })
}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut main_fut = std::pin::pin!(future);
    let signal = Arc::new(Signal::new());
    let runnable: Mutex<VecDeque<Arc<Task>>> = Mutex::new(VecDeque::with_capacity(1024));
    let waker = Waker::from(signal.clone());
    let mut cx = Context::from_waker(&waker);
    SIGNAL.set(&signal, || {
        RUNNABLE.set(&runnable, || {
            loop {
                if let Ready(output) = main_fut.as_mut().poll(&mut cx) {
                    return output;
                }
                while let Some(task) = runnable.lock().unwrap().pop_front() {
                    let waker = Waker::from(task.clone());
                    let mut cx = Context::from_waker(&waker);
                    let _ = task.future.borrow_mut().as_mut().poll(&mut cx);
                    task.signal.wait();
                }
                signal.wait();
            }
        })
    })
}
fn spawn<F: Future<Output = ()> + std::marker::Send + 'static>(future: F) {
    let signal = Arc::new(Signal::new());
    let task = Arc::new(Task {
        future: RefCell::new(Box::pin(future)),
        signal: signal.clone(),
    });
    RUNNABLE.with(|runnable| {
        runnable.lock().unwrap().push_back(task.clone());
        task.signal.notify();
    });
}
async fn demo() {
    let (tx, rx) = async_channel::bounded::<()>(1);

    spawn(demo2(tx));
    println!("hello world!");
    let _ = rx.recv().await;

    std::thread::sleep(Duration::from_secs(2));
    println!("demo1 wake. Ending...");
}

async fn demo2(tx: async_channel::Sender<()>) {
    println!("Hello World 2");
    let _ = tx.send(()).await;
    let (tx, rx) = async_channel::bounded::<()>(1);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        tx.send(());
    });
    let _ = rx.recv().await;
    println!("demo2 waked.");
}
fn main() {

    block_on(demo());
}

