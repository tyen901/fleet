use crate::ui::screen::Screen;

pub enum NavOp {
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
    PopToRoot,
    CloseApp,
}

pub trait Navigation {
    fn push(&mut self, screen: Box<dyn Screen>);
    fn pop(&mut self);
    fn replace(&mut self, screen: Box<dyn Screen>);
    fn pop_to_root(&mut self);
    fn close_app(&mut self);
}

pub struct NavHost {
    ops: Vec<NavOp>,
}

impl NavHost {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn take_ops(&mut self) -> Vec<NavOp> {
        std::mem::take(&mut self.ops)
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

/// Screen factory trait (keeps nav ops screen-only).
pub trait Screens: Send + Sync {
    fn list(&self) -> Box<dyn Screen>;
    fn detail(&self, id: &str) -> Box<dyn Screen>;
    fn form_new(&self) -> Box<dyn Screen>;
    fn form_edit(&self, id: &str) -> Box<dyn Screen>;
    fn settings(&self) -> Box<dyn Screen>;
}
