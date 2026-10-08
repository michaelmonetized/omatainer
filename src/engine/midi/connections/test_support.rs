//! Controllable backend exercising the production manager and raw input sink.
use super::*;
use crate::engine::{midi::MidiHub, Engine};
use crossbeam_channel::{Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(crate) struct Input(Arc<Mutex<Option<handoff::InputSink>>>);
impl Input {
    pub fn push(&self, bytes: &[u8]) {
        self.0.lock().as_mut().unwrap().push(bytes);
    }
    pub fn is_closed(&self) -> bool {
        self.0.lock().is_none()
    }
}

pub(crate) enum Attempt {
    Discover,
    Connect { id: String, input: Input },
}
pub(crate) enum Reply {
    Ports(Result<Vec<(String, String)>, String>),
    Connected(Result<(), String>),
}
pub(crate) struct Control {
    pub presence:Arc<Mutex<Option<Vec<(String,String)>>>>,
    pub attempts: Receiver<Attempt>,
    pub replies: Sender<Reply>,
}
impl Control {
    pub fn next(&self) -> Attempt {
        self.attempts.recv_timeout(Duration::from_secs(3)).unwrap()
    }
    pub fn discover(&self, ports: &[(&str, &str)]) {
        assert!(matches!(self.next(), Attempt::Discover));
        self.replies
            .send(Reply::Ports(Ok(ports
                .iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect())))
            .unwrap();
    }
    pub fn connect(&self, id: &str, result: Result<(), &str>) -> Input {
        let Attempt::Connect { id: actual, input } = self.next() else {
            panic!("expected connection")
        };
        assert_eq!(actual, id);
        self.replies
            .send(Reply::Connected(result.map_err(str::to_string)))
            .unwrap();
        input
    }
}
struct Fake {
    presence:Arc<Mutex<Option<Vec<(String,String)>>>>,
    registry:Option<Arc<super::super::catalog::runtime::Registry>>,
    attempts: Sender<Attempt>,
    replies: Receiver<Reply>,
}
struct Connection(Input);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0 .0.lock().take();
    }
}
impl Backend for Fake {
    type Port = String;
    type Connection = Connection;
    fn discover(&mut self) -> Result<Vec<Port<String>>, String> {
        self.attempts
            .send(Attempt::Discover)
            .map_err(|_| "fixture closed")?;
        let Reply::Ports(result) = self.replies.recv().map_err(|_| "fixture closed")? else {
            panic!("expected discovery reply")
        };
        result.map(|ports| {
            ports
                .into_iter()
                .map(|(id, name)| Port {
                    port: id.clone(),
                    id,
                    name,
                })
                .collect()
        })
    }
    fn presence(&mut self)->Result<Option<Vec<(String,String)>>,String>{Ok(self.presence.lock().clone())}
    fn mapping(&self,id:&str,name:&str,maps:&[MidiMap])->MidiMap{self.registry.as_ref().map_or_else(||pick_map(maps,name),|r|r.mapping(id,None))}
    fn endpoint(&self,id:&str,name:&str,policy:&InputPolicy)->(String,String,String){self.registry.as_ref().map_or_else(||(name.into(),id.into(),name.into()),|r|r.endpoint(id,name,policy))}
    fn incarnation(&self,id:&str)->String{self.registry.as_ref().and_then(|r|r.view().devices.iter().find(|d|d.id==id).map(|d|d.device.connection.clone())).unwrap_or_else(||id.into())}
    fn registry(&self)->Option<Arc<super::super::catalog::runtime::Registry>>{self.registry.clone()}
    fn connect(
        &mut self,
        id: &String,
        _name: &str,
        input: handoff::InputSink,
    ) -> Result<Connection, String> {
        let input = Input(Arc::new(Mutex::new(Some(input))));
        let connection = Connection(input.clone());
        self.attempts
            .send(Attempt::Connect {
                id: id.clone(),
                input,
            })
            .map_err(|_| "fixture closed")?;
        let Reply::Connected(result) = self.replies.recv().map_err(|_| "fixture closed")? else {
            panic!("expected connect reply")
        };
        result.map(|()| connection)
    }
}
pub(crate) fn install(engine: &mut Engine) -> Control {
    install_with_policy(engine, InputPolicy::All)
}
pub(crate) fn install_with_policy(engine: &mut Engine, policy: InputPolicy) -> Control {
    install_profiled(engine,policy,None)
}
pub(crate) fn install_profiled(engine:&mut Engine,policy:InputPolicy,registry:Option<Arc<super::super::catalog::runtime::Registry>>)->Control{
    let (attempt_tx, attempts) = bounded(8);
    let (replies, reply_rx) = bounded(8);
    let presence=Arc::new(Mutex::new(None));
    let mut hub = MidiHub::without_devices();
    hub.profiles=registry.clone();
    hub.connections = Some(
        Manager::start_with_policy(
            Fake {
                presence:presence.clone(),registry,
                attempts: attempt_tx,
                replies: reply_rx,
            },
            &engine.snap,
            engine.cmd.clone(),
            super::super::builtin_maps().unwrap(),
            hub.log.clone(),
            hub.input_counters.clone(),
            policy,
        )
        .unwrap(),
    );
    engine.midi = hub;
    Control { presence,attempts, replies }
}
pub(crate) fn until(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < end, "connection state did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}
