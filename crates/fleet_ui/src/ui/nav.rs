use crate::ui::screen::Screen;
use parking_lot::Mutex;
use std::sync::Arc;

pub trait Navigation {
    fn push(&mut self, screen: Box<dyn Screen>);
    fn pop(&mut self);
    fn replace(&mut self, screen: Box<dyn Screen>);
    fn pop_to_root(&mut self);
    fn close_app(&mut self);
}

pub trait Screens: Send + Sync {
    fn hub(&self) -> Box<dyn Screen>;
    fn dashboard(&self) -> Box<dyn Screen>;
    fn editor_new(&self) -> Box<dyn Screen>;
    fn editor_edit(&self, id: String) -> Box<dyn Screen>;
    fn settings(&self) -> Box<dyn Screen>;
}

pub(crate) enum NavOp {
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
    PopToRoot,
    CloseApp,
}

pub struct NavHost {
    ops: Vec<NavOp>,
    close_requested: bool,
}

impl NavHost {
    pub fn new() -> Self {
        Self {
            ops: Vec::new(),
            close_requested: false,
        }
    }

    pub fn take_ops(&mut self) -> Vec<NavOp> {
        std::mem::take(&mut self.ops)
    }

    pub fn close_requested(&self) -> bool {
        self.close_requested
    }
}

impl Default for NavHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Navigation for NavHost {
    fn push(&mut self, screen: Box<dyn Screen>) {
        self.ops.push(NavOp::Push(screen));
    }

    fn pop(&mut self) {
        self.ops.push(NavOp::Pop);
    }

    fn replace(&mut self, screen: Box<dyn Screen>) {
        self.ops.push(NavOp::Replace(screen));
    }

    fn pop_to_root(&mut self) {
        self.ops.push(NavOp::PopToRoot);
    }

    fn close_app(&mut self) {
        self.ops.push(NavOp::CloseApp);
    }
}

// --- testing harness helper (nav collector)

#[derive(Default)]
pub struct NavCollector {
    ops: Arc<Mutex<Vec<String>>>,
}

impl NavCollector {
    pub fn ops(&self) -> Vec<String> {
        self.ops.lock().clone()
    }

    pub fn take_ops(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.ops.lock())
    }
}

impl Navigation for NavCollector {
    fn push(&mut self, screen: Box<dyn Screen>) {
        self.ops.lock().push(format!("push:{}", screen.name()));
    }
    fn pop(&mut self) {
        self.ops.lock().push("pop".into());
    }
    fn replace(&mut self, screen: Box<dyn Screen>) {
        self.ops.lock().push(format!("replace:{}", screen.name()));
    }
    fn pop_to_root(&mut self) {
        self.ops.lock().push("pop_to_root".into());
    }
    fn close_app(&mut self) {
        self.ops.lock().push("close_app".into());
    }
}
