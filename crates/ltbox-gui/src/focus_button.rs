//! Keyboard interaction for Iced's pointer-only controls.
use crate::{Message, pal_of};
use iced::advanced::widget::{Operation, Tree, operation::Focusable, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
pub use iced::widget::button::{Status, Style};
use iced::{Element, Event, Length, Padding, Rectangle, Renderer, Size, Theme, Vector, keyboard};

pub(crate) struct Button<'a, M = Message> {
    inner: iced::widget::Button<'a, M>,
    action: Option<M>,
}

pub fn button<'a>(content: impl Into<Element<'a, Message>>) -> Button<'a> {
    Button {
        inner: iced::widget::button(content),
        action: None,
    }
}

impl<'a> Button<'a> {
    pub fn on_press(mut self, message: Message) -> Self {
        self.action = Some(message.clone());
        self.inner = self.inner.on_press(message);
        self
    }
    pub fn on_press_maybe(mut self, message: Option<Message>) -> Self {
        self.action = message.clone();
        self.inner = self.inner.on_press_maybe(message);
        self
    }
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }
}

impl<'a> From<Button<'a>> for Element<'a, Message> {
    fn from(button: Button<'a>) -> Self {
        actionable(button.inner, button.action)
    }
}

/// Also used for native checkboxes; their pointer behavior stays native.
pub fn actionable<'a>(
    inner: impl Into<Element<'a, Message>>,
    action: Option<Message>,
) -> Element<'a, Message> {
    Element::new(Interaction {
        inner: inner.into(),
        action,
        inert: false,
        cycle: None,
    })
}

/// Keep obscured layers out of keyboard traversal and activation.
pub fn scope(inner: Element<'_, Message>, enabled: bool) -> Element<'_, Message> {
    Element::new(Interaction {
        inner,
        action: None,
        inert: !enabled,
        cycle: None,
    })
}

#[derive(Default)]
struct State {
    focused: bool,
    armed: Option<keyboard::key::Named>,
    key: Option<String>,
}
impl State {
    fn press(&mut self, key: keyboard::key::Named) {
        if self.focused {
            self.armed = Some(key);
        }
    }
    fn release(&mut self, key: keyboard::key::Named) -> bool {
        self.focused && self.armed.take() == Some(key)
    }
}
impl Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
        self.armed = None;
    }
}

struct Interaction<'a> {
    inner: Element<'a, Message>,
    action: Option<Message>,
    inert: bool,
    cycle: Option<(String, Message)>,
}

/// Native pick lists retain mouse menus; arrows cycle choices on the keyboard.
pub fn cycle<'a, T: PartialEq + Clone>(
    inner: impl Into<Element<'a, Message>>,
    id: String,
    options: &[T],
    selected: &T,
    on_change: impl Fn(T) -> Message,
) -> Element<'a, Message> {
    if options.is_empty() {
        return inner.into();
    }
    let index = options
        .iter()
        .position(|value| value == selected)
        .unwrap_or(0);
    let previous = on_change(options[(index + options.len() - 1) % options.len()].clone());
    let next = on_change(options[(index + 1) % options.len()].clone());
    Element::new(Interaction {
        inner: inner.into(),
        action: Some(next),
        inert: false,
        cycle: Some((id, previous)),
    })
}

impl Interaction<'_> {
    fn identity(&self) -> Option<String> {
        if matches!(
            self.action,
            Some(Message::Settings(crate::SettingsMsg::SetUseSystemFont(_)))
        ) {
            return Some("system-font-switch".into());
        }
        if matches!(self.action, Some(Message::StartupDisclaimerToggled(_))) {
            return Some("startup-acknowledgement".into());
        }
        self.cycle
            .as_ref()
            .map(|(id, _)| id.clone())
            .or_else(|| self.action.as_ref().map(|m| format!("{m:?}")))
    }
}

/// Reveal the focused control in its nearest scrolling ancestor after Tab.
pub fn reveal_focus() -> iced::Task<Message> {
    iced::advanced::widget::operate(Reveal::default())
}

#[derive(Default)]
struct Reveal {
    next: usize,
    pending: Option<(usize, Rectangle, Vector)>,
    ancestors: Vec<(usize, Rectangle, Vector)>,
    target: Option<(usize, f32)>,
}
impl<T: Send + 'static> Operation<T> for Reveal {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation<T>)) {
        let pushed = self.pending.take();
        if let Some(parent) = pushed {
            self.ancestors.push(parent);
        }
        visit(self);
        if pushed.is_some() {
            self.ancestors.pop();
        }
    }
    fn scrollable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        bounds: Rectangle,
        _: Rectangle,
        translation: Vector,
        _: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        self.pending = Some((self.next, bounds, translation));
        self.next += 1;
    }
    fn focusable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        bounds: Rectangle,
        state: &mut dyn Focusable,
    ) {
        if state.is_focused()
            && let Some(&(index, viewport, offset)) = self.ancestors.last()
        {
            let top = bounds.y - offset.y;
            let bottom = top + bounds.height;
            let delta = if top < viewport.y {
                top - viewport.y
            } else if bottom > viewport.y + viewport.height {
                bottom - viewport.y - viewport.height
            } else {
                0.0
            };
            if delta != 0.0 {
                self.target = Some((index, (offset.y + delta).max(0.0)));
            }
        }
    }
    fn finish(&self) -> iced::advanced::widget::operation::Outcome<T> {
        match self.target {
            Some((index, y)) => {
                iced::advanced::widget::operation::Outcome::Chain(Box::new(ScrollFocus {
                    index,
                    y,
                    current: 0,
                }))
            }
            None => iced::advanced::widget::operation::Outcome::None,
        }
    }
}
struct ScrollFocus {
    index: usize,
    y: f32,
    current: usize,
}
impl<T> Operation<T> for ScrollFocus {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation<T>)) {
        visit(self);
    }
    fn scrollable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        _: Rectangle,
        _: Rectangle,
        _: Vector,
        state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if self.current == self.index {
            state.scroll_to(
                iced::advanced::widget::operation::scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(self.y),
                },
            );
        }
        self.current += 1;
    }
}
impl Widget<Message, Theme, Renderer> for Interaction<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            key: self.identity(),
            ..Default::default()
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner)]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let key = self.identity();
        if state.key != key || self.inert {
            state.unfocus();
            state.key = key;
        }
        tree.diff_children(std::slice::from_ref(&self.inner));
    }
    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        if self.inert {
            return;
        }
        if self.action.is_some() {
            operation.focusable(None, layout.bounds(), tree.state.downcast_mut::<State>());
        }
        self.inner
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.inert && matches!(event, Event::Keyboard(_)) {
            return;
        }
        let state = tree.state.downcast_mut::<State>();
        if !shell.is_event_captured() {
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            ) {
                state.unfocus();
            }
            if state.focused
                && let Some(action) = &self.action
            {
                if let Some((_, previous)) = &self.cycle
                    && let Event::Keyboard(keyboard::Event::KeyPressed {
                        key: keyboard::Key::Named(key),
                        ..
                    }) = event
                {
                    match key {
                        keyboard::key::Named::ArrowUp | keyboard::key::Named::ArrowLeft => {
                            shell.publish(previous.clone());
                            shell.capture_event();
                            return;
                        }
                        keyboard::key::Named::ArrowDown | keyboard::key::Named::ArrowRight => {
                            shell.publish(action.clone());
                            shell.capture_event();
                            return;
                        }
                        _ => {}
                    }
                }
                match event {
                    Event::Keyboard(keyboard::Event::KeyPressed {
                        key:
                            keyboard::Key::Named(
                                key @ (keyboard::key::Named::Enter | keyboard::key::Named::Space),
                            ),
                        ..
                    }) => {
                        state.press(*key);
                        shell.capture_event();
                        return;
                    }
                    Event::Keyboard(keyboard::Event::KeyReleased {
                        key:
                            keyboard::Key::Named(
                                key @ (keyboard::key::Named::Enter | keyboard::key::Named::Space),
                            ),
                        ..
                    }) => {
                        if state.release(*key) {
                            shell.publish(action.clone());
                        }
                        shell.capture_event();
                        return;
                    }
                    _ => {}
                }
            }
        }
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if !self.inert && self.action.is_some() && tree.state.downcast_ref::<State>().focused {
            use iced::advanced::Renderer as _;
            let mut bounds = layout.bounds();
            bounds.x += 2.0;
            bounds.y += 2.0;
            bounds.width = (bounds.width - 4.0).max(0.0);
            bounds.height = (bounds.height - 4.0).max(0.0);
            // Contrasting backing keeps the ring visible on filled actions too.
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: iced::Border {
                        color: pal_of(theme).surface,
                        width: 5.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                },
                iced::Color::TRANSPARENT,
            );
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: iced::Border {
                        color: pal_of(theme).primary,
                        width: 3.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                },
                iced::Color::TRANSPARENT,
            );
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        if self.inert {
            return None;
        }
        self.inner.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_or_disabling_an_action_clears_keyboard_focus() {
        let original = actionable(
            iced::widget::Space::new(),
            Some(Message::Navigate(crate::View::Root)),
        );
        let mut tree = Tree::new(&original);
        tree.state.downcast_mut::<State>().focus();
        let mut replacement = actionable(
            iced::widget::Space::new(),
            Some(Message::Navigate(crate::View::Reboot)),
        );
        replacement.as_widget_mut().diff(&mut tree);
        assert!(!tree.state.downcast_ref::<State>().is_focused());
        tree.state.downcast_mut::<State>().focus();
        let mut disabled = actionable(iced::widget::Space::new(), None);
        disabled.as_widget_mut().diff(&mut tree);
        assert!(!tree.state.downcast_ref::<State>().is_focused());
    }

    #[test]
    fn revealing_focus_scrolls_only_when_outside_the_viewport() {
        let mut state = State::default();
        state.focus();
        let mut reveal = Reveal {
            ancestors: vec![(
                2,
                Rectangle {
                    x: 0.0,
                    y: 100.0,
                    width: 80.0,
                    height: 200.0,
                },
                Vector::new(0.0, 40.0),
            )],
            ..Default::default()
        };
        Operation::<()>::focusable(
            &mut reveal,
            None,
            Rectangle {
                x: 0.0,
                y: 180.0,
                width: 56.0,
                height: 64.0,
            },
            &mut state,
        );
        assert_eq!(reveal.target, None);
        Operation::<()>::focusable(
            &mut reveal,
            None,
            Rectangle {
                x: 0.0,
                y: 330.0,
                width: 56.0,
                height: 64.0,
            },
            &mut state,
        );
        assert_eq!(reveal.target, Some((2, 94.0)));
    }

    #[test]
    fn keyboard_repeat_activates_once_on_release() {
        let mut state = State::default();
        state.focus();
        state.press(keyboard::key::Named::Enter);
        state.press(keyboard::key::Named::Enter);
        assert!(state.release(keyboard::key::Named::Enter));
        assert!(!state.release(keyboard::key::Named::Enter));
    }
    #[test]
    fn changing_focus_cancels_pending_activation() {
        let mut state = State::default();
        state.focus();
        state.press(keyboard::key::Named::Space);
        state.unfocus();
        state.focus();
        assert!(!state.release(keyboard::key::Named::Space));
    }
}
