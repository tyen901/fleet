use crate::ui::screen::Screen;

pub enum NavOp {
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
    PopToRoot,
    CloseApp,
}

#[derive(Default)]
pub struct NavHost {
    ops: Vec<NavOp>,
}

impl NavHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, screen: Box<dyn Screen>) {
        self.ops.push(NavOp::Push(screen));
    }

    pub fn pop(&mut self) {
        self.ops.push(NavOp::Pop);
    }

    pub fn replace(&mut self, screen: Box<dyn Screen>) {
        self.ops.push(NavOp::Replace(screen));
    }

    pub fn pop_to_root(&mut self) {
        self.ops.push(NavOp::PopToRoot);
    }

    pub fn close_app(&mut self) {
        self.ops.push(NavOp::CloseApp);
    }

    pub fn take_ops(&mut self) -> Vec<NavOp> {
        std::mem::take(&mut self.ops)
    }
}

/// A registry of screens that can be navigated to.
pub trait Screens: Send + Sync {
    fn hub(&self) -> Box<dyn Screen>;
    fn dashboard(&self) -> Box<dyn Screen>;
    fn editor_new(&self) -> Box<dyn Screen>;
    fn editor_edit(&self, id: &str) -> Box<dyn Screen>;
    fn settings(&self) -> Box<dyn Screen>;
}

unsafe impl Send for NavHost {}
unsafe impl Sync for NavHost {}
