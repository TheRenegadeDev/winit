//! Repeat provenance from the event-ordered keymap following a core FocusIn.
//!
//! X11 sends KeymapNotify immediately after FocusIn for KeymapState subscribers.
//! Unlike a later QueryKeymap reply, that event still describes the focus boundary
//! when a repeat and its release are already queued by the time dispatch resumes.
//! See https://www.x.org/releases/X11R7.5/doc/libX11/libX11.html, section 10.8.

#[derive(Default)]
pub(super) struct FocusKeymap {
    pending: Option<u32>,
    window: Option<u32>,
    keys: [u8; 32],
}

impl FocusKeymap {
    pub fn focus_in(&mut self, window: u32) {
        self.pending = Some(window);
        self.window = None;
        self.keys = [0; 32];
    }

    pub fn focus_out(&mut self, window: u32) {
        if self.pending == Some(window) {
            self.pending = None;
        }
        if self.window == Some(window) {
            self.window = None;
            self.keys = [0; 32];
        }
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    pub fn keymap_notify(&mut self, keys: [u8; 32]) -> Option<u32> {
        let window = self.pending.take()?;
        self.window = Some(window);
        self.keys = keys;
        Some(window)
    }

    pub fn key_event(&mut self, window: u32, keycode: u32, pressed: bool) -> bool {
        if self.window != Some(window) || !(8..=255).contains(&keycode) {
            return false;
        }
        let Some(byte) = self.keys.get_mut(keycode as usize / 8) else { return false };
        let mask = 1 << (keycode % 8);
        let was_held = *byte & mask != 0;
        if !pressed {
            *byte &= !mask;
        }
        pressed && was_held
    }

    pub fn key_event_from(
        &mut self,
        active: u32,
        source: u32,
        keycode: u32,
        pressed: bool,
    ) -> Option<bool> {
        if active != source {
            return None;
        }
        Some(self.key_event(active, keycode, pressed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 10;
    const B: u32 = 20;
    const N: u32 = 57;
    const Y: u32 = 29;

    fn down(key: u32) -> [u8; 32] {
        let mut keys = [0; 32];
        keys[key as usize / 8] |= 1 << (key % 8);
        keys
    }

    fn late_query_does_not_erase_the_focus_event_state(key: u32) {
        let mut state = FocusKeymap::default();
        state.focus_in(B);
        // Server queue: FocusIn, KeymapNotify(held), auto-repeat, physical release.
        // By the time client dispatch starts, a fresh QueryKeymap would report all keys up.
        let event_ordered_keys = down(key);
        let late_query_keys = [0; 32];
        assert_ne!(event_ordered_keys, late_query_keys);
        assert_eq!(state.keymap_notify(event_ordered_keys), Some(B));
        assert!(state.key_event(B, key, true), "the queued auto-repeat belongs to the old hold");
        assert!(!state.key_event(B, key, false));
        assert!(!state.key_event(B, key, true), "a later deliberate gesture stays fresh");
    }

    #[test]
    fn delayed_n_release_before_query_keeps_the_queued_repeat_identity() {
        late_query_does_not_erase_the_focus_event_state(N);
    }

    #[test]
    fn delayed_y_release_before_query_keeps_the_queued_repeat_identity() {
        late_query_does_not_erase_the_focus_event_state(Y);
    }

    #[test]
    fn fresh_press_after_an_unseen_release_is_not_a_repeat() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.keymap_notify(down(N));
        state.focus_out(A);
        state.focus_in(B);
        state.keymap_notify([0; 32]);
        assert!(!state.key_event(B, N, true));
    }

    #[test]
    fn release_without_any_repeat_allows_the_next_fresh_press() {
        for key in [N, Y] {
            let mut state = FocusKeymap::default();
            state.focus_in(B);
            state.keymap_notify(down(key));
            assert!(!state.key_event(B, key, false));
            assert!(!state.key_event(B, key, true));
        }
    }

    #[test]
    fn unrelated_window_and_pointer_keymaps_do_not_retarget_provenance() {
        let mut state = FocusKeymap::default();
        assert_eq!(state.keymap_notify(down(N)), None, "pointer-enter keymaps are not focus state");
        state.focus_in(A);
        assert_eq!(state.keymap_notify(down(N)), Some(A));
        assert_eq!(state.keymap_notify([0; 32]), None);
        assert!(!state.key_event(B, N, true));
        assert!(!state.key_event(B, N, false));
        state.focus_out(B);
        assert!(state.key_event(A, N, true));
        state.focus_out(A);
        assert!(!state.key_event(A, N, true));
    }

    #[test]
    fn losing_focus_before_the_snapshot_cancels_it() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.focus_out(A);
        assert_eq!(state.keymap_notify(down(N)), None);
        assert!(!state.key_event(A, N, true));
    }

    #[test]
    fn an_untracked_focus_event_cancels_pending_snapshot_authority() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.cancel_pending();
        assert_eq!(state.keymap_notify(down(N)), None);
        assert!(!state.key_event(A, N, true));
    }

    #[test]
    fn pointer_entry_cancels_stale_pending_but_does_not_erase_an_owned_hold() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.cancel_pending();
        assert_eq!(state.keymap_notify(down(Y)), None);

        state.focus_in(A);
        assert_eq!(state.keymap_notify(down(N)), Some(A));
        state.cancel_pending();
        assert_eq!(state.keymap_notify([0; 32]), None);
        assert!(state.key_event(A, N, true));
    }

    #[test]
    fn an_unrelated_windows_release_cannot_end_the_active_windows_hold() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.keymap_notify(down(N));
        assert_eq!(state.key_event_from(A, B, N, false), None);
        assert_eq!(state.key_event_from(A, A, N, true), Some(true));
    }

    #[test]
    fn a_delayed_old_windows_key_cannot_become_a_fresh_press_in_the_new_window() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.keymap_notify(down(N));
        state.focus_out(A);
        state.focus_in(B);
        state.keymap_notify([0; 32]);
        assert_eq!(state.key_event_from(B, A, N, true), None);
        assert_eq!(state.key_event_from(B, A, N, false), None);
        assert_eq!(state.key_event_from(B, B, N, true), Some(false));
    }

    #[test]
    fn out_of_range_keycodes_are_not_repeats() {
        let mut state = FocusKeymap::default();
        state.focus_in(A);
        state.keymap_notify([255; 32]);
        assert!(!state.key_event(A, u32::MAX, true));
    }
}
