//! The window-only lead of `Game::tick`.
use super::{Game, Screen};
use horizon_client::{Result, text::Text};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Memory is read this often.
const MEMORY_CHECK: Duration = Duration::from_secs(5);
/// A warning repeats no sooner than this while memory stays low.
const MEMORY_REPEAT: Duration = Duration::from_secs(120);
/// Memory is low below this many free megabytes, or below this share of the total.
const LOW_MB: u64 = 500;
const LOW_SHARE: u64 = 8;

/// When memory was last read and when it last warned.
static MEMORY_STATE: Mutex<(Option<Instant>, Option<Instant>)> = Mutex::new((None, None));

/// Whether this much free memory is low.
fn memory_is_low(available_mb: u64, total_mb: u64) -> bool {
    available_mb < LOW_MB.min(total_mb * LOW_SHARE / 100).max(1)
}

impl Game<'_> {
    /// Warns in the chat when the device's memory is low while playing: the system ends apps that
    /// use too much, and the game with them.
    fn tick_low_memory(&mut self) {
        let now = Instant::now();
        {
            let Ok(mut state) = MEMORY_STATE.lock() else {
                return;
            };
            if state.0.is_some_and(|at| now.duration_since(at) < MEMORY_CHECK) {
                return;
            }
            state.0 = Some(now);
        }
        let Some(info) = crate::entry::memory_info() else {
            return;
        };
        if !memory_is_low(info.available_mb, info.total_mb) {
            return;
        }
        {
            let Ok(mut state) = MEMORY_STATE.lock() else {
                return;
            };
            if state.1.is_some_and(|at| now.duration_since(at) < MEMORY_REPEAT) {
                return;
            }
            state.1 = Some(now);
        }
        log::warn!(
            "low memory: {} MB free of {} MB",
            info.available_mb,
            info.total_mb
        );
        self.debug_warning(Text::translate(
            "horizon.memory.low",
            &[Text::literal(info.available_mb.to_string())],
        ));
    }
    pub(super) fn tick_client_lead(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        self.tick_low_memory();
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                if let Some(c) = self.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.set_screen(Screen::Inventory);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::memory_is_low;

    #[test]
    fn low_memory_is_a_few_hundred_megabytes_or_a_small_share() {
        assert!(memory_is_low(300, 12_000));
        assert!(!memory_is_low(2_000, 12_000));
        // A small device: the share counts, not the fixed amount.
        assert!(!memory_is_low(300, 2_000));
        assert!(memory_is_low(100, 2_000));
    }
}
