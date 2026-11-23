//! Options > Horizon Settings > Debug: things for testing the app that are only in the debug build
//! (`horizon-android-debug`): crashing the game on purpose, to try the crash screen.
use horizon_hooks::pages::{Act, Page, Place, Row};

/// The page's text.
pub static LANG: &horizon_hooks::lang::Files = &[("en_us", include_str!("../assets/lang/debug_en_us.json"))];

pub static PAGE: Debug = Debug;
pub struct Debug;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Display,
    CrashError,
    CrashPanic,
}

fn layout() -> Vec<(Row, Item)> {
    vec![
        (Row::Heading("horizon.android.debug.crash"), Item::Display),
        (
            Row::button("horizon.android.debug.crash_error"),
            Item::CrashError,
        ),
        (
            Row::text(
                "Stops the game with an error, as a bug would: it saves a crash report, and the game starts again on the crash screen.",
            )
            .quiet(),
            Item::Display,
        ),
        (
            Row::button("horizon.android.debug.crash_panic"),
            Item::CrashPanic,
        ),
        (
            Row::text("Panics on the game's thread, as a bug in the game's code would: the same crash report and restart.")
                .quiet(),
            Item::Display,
        ),
    ]
}

impl Page for Debug {
    fn id(&self) -> &'static str {
        "horizon-android.debug"
    }
    fn title(&self) -> &'static str {
        "horizon.android.debug"
    }
    fn place(&self) -> Place {
        Place::HorizonSettings
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.extend(layout().into_iter().map(|(row, _)| row));
    }
    fn act(&self, row: usize, act: Act) {
        let Some((_, item)) = layout().into_iter().nth(row) else {
            return;
        };
        match (item, act) {
            // An error the game stops with, as a bug's: the crash report is saved and the game ends.
            (Item::CrashError, Act::Press) => horizon_client::entry::request_crash(),
            (Item::CrashPanic, Act::Press) => panic!("Debug: the panic button was pressed"),
            _ => {}
        }
    }
}
