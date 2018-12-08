use std::io;
use std::fmt;
use std::str;
use std::error::Error;
use std::result::Result;
use std::sync::atomic::Ordering;
use caseless;
use futures::{future, Future};
use futures::sync::oneshot;
use atomic_option::AtomicOption;
use protocol::{Resp, BulkStr, BinSafeStr, Array};

#[derive(Debug, PartialEq, Clone)]
pub enum CmdType {
    Ping,
    Info,
    Auth,
    Quit,
    Echo,
    Select,
    Others,
    Invalid,
    UmCtl,
    Cluster,
}

#[derive(Debug)]
pub struct Command {
    request: Resp
}

impl Command {
    pub fn new(request: Resp) -> Self {
        Command{
            request: request,
        }
    }

    pub fn get_resp(&self) -> &Resp {
        &self.request
    }

    pub fn get_type(&self) -> CmdType {
        let resps = match self.request {
            Resp::Arr(Array::Arr(ref resps)) => resps,
            _ => return CmdType::Invalid,
        };

        let first_resp = resps.first();
        let resp = match first_resp {
            Some(ref resp) => resp,
            None => return CmdType::Invalid,
        };

        let first = match resp {
            Resp::Bulk(BulkStr::Str(ref first)) => first,
            _ => return CmdType::Invalid,
        };

        let cmd_name = match str::from_utf8(first) {
            Ok(cmd_name) => cmd_name,
            Err(_) => return CmdType::Invalid,
        };

        if caseless::canonical_caseless_match_str(cmd_name, "PING") {
            CmdType::Ping
        } else if caseless::canonical_caseless_match_str(cmd_name, "INFO") {
            CmdType::Info
        } else if caseless::canonical_caseless_match_str(cmd_name, "Auth") {
            CmdType::Auth
        } else if caseless::canonical_caseless_match_str(cmd_name, "Quit") {
            CmdType::Quit
        } else if caseless::canonical_caseless_match_str(cmd_name, "Echo") {
            CmdType::Echo
        } else if caseless::canonical_caseless_match_str(cmd_name, "Select") {
            CmdType::Select
        } else if caseless::canonical_caseless_match_str(cmd_name, "UmCtl") {
            CmdType::UmCtl
        } else if caseless::canonical_caseless_match_str(cmd_name, "Cluster") {
            CmdType::Cluster
        } else {
            CmdType::Others
        }
    }

    pub fn get_key(&self) -> Option<BinSafeStr> {
        get_key(self.get_resp())
    }
}

pub fn get_key(resp :&Resp) -> Option<BinSafeStr> {
    match resp {
        Resp::Arr(Array::Arr(ref resps)) => {
            resps.get(1).and_then(|resp| {
                match resp {
                    Resp::Bulk(BulkStr::Str(ref s)) => Some(s.clone()),
                    Resp::Simple(ref s) => Some(s.clone()),
                    _ => None,
                }
            })
        },
        _ => None,
    }
}

pub struct CmdReplySender {
    cmd: Command,
    reply_sender: AtomicOption<oneshot::Sender<CommandResult>>,
}

impl fmt::Debug for CmdReplySender {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "cmd: {:?}", self.cmd)
    }
}

pub struct CmdReplyReceiver {
    reply_receiver: oneshot::Receiver<CommandResult>,
}

pub fn new_command_pair(cmd: Command) -> (CmdReplySender, CmdReplyReceiver) {
    let (s, r) = oneshot::channel::<CommandResult>();
    let reply_sender = CmdReplySender{
        cmd: cmd,
        reply_sender: AtomicOption::new(Box::new(s)),
    };
    let reply_receiver = CmdReplyReceiver{
        reply_receiver: r,
    };
    (reply_sender, reply_receiver)
}

impl CmdReplySender {
    pub fn get_cmd(&self) -> &Command {
        &self.cmd
    }

    pub fn send(&self, res: CommandResult) -> Result<(), CommandError> {
        // Must not send twice.
        self.reply_sender.take(Ordering::SeqCst).unwrap().send(res)
            .map_err(|_| CommandError::Canceled)
    }

    pub fn try_send(&self, res: CommandResult) -> Option<Result<(), CommandError>> {
        match self.reply_sender.take(Ordering::SeqCst) {
            Some(reply_sender) => Some(reply_sender.send(res)
                .map_err(|_| CommandError::Canceled)),
            None => None,
        }
    }
}

impl CmdReplyReceiver {
    pub fn wait_response(self) -> impl Future<Item = Resp, Error = CommandError> + Send {
        self.reply_receiver
            .map_err(|_| CommandError::Canceled)
            .and_then(|result: CommandResult| {
                future::result(result)
            })
    }
}

#[derive(Debug)]
pub enum CommandError {
    Io(io::Error),
    UnexpectedResponse,
    Dropped,
    Canceled,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for CommandError {
    fn description(&self) -> &str {
        "command error"
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            CommandError::Io(err) => Some(err),
            _ => None,
        }
    }
}

pub type CommandResult = Result<Resp, CommandError>;
