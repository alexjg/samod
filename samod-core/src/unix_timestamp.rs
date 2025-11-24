use std::{
    ops::{Add, AddAssign, Sub},
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
use js_sys;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixTimestamp {
    millis: u128,
}

impl std::fmt::Display for UnixTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.millis)
    }
}

impl std::fmt::Debug for UnixTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.millis)
    }
}

impl UnixTimestamp {
    pub fn now() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            // Try to get external time provider first
            if let Some(millis) = crate::time_provider::get_external_time() {
                return Self { millis };
            }

            let result = std::panic::catch_unwind(|| js_sys::Date::now());

            match result {
                Ok(millis) => Self {
                    millis: millis as u128,
                },
                Err(_) => {
                    panic!(
                        "Cannot access Date.now() in this WASM environment!\n\
                   \n\
                   If you're using Node.js or another WASI runtime, you need to provide \n\
                   a time function. Example for Node.js:\n\
                   \n\
                   ```javascript\n\
                   import init, {{ set_time_provider }} from './samod_core.js';\n\
                   \n\
                   await init();\n\
                   set_time_provider(() => Date.now());\n\
                   ```\n\
                   \n\
                   See https://github.com/alexjg/samod#wasi-environments for more details."
                    )
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        Self {
            millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        }
    }

    pub fn from_millis(millis: u128) -> Self {
        Self { millis }
    }

    pub fn as_millis(&self) -> u128 {
        self.millis
    }
}

impl From<UnixTimestamp> for i64 {
    fn from(ts: UnixTimestamp) -> i64 {
        ts.millis as i64
    }
}

impl AddAssign<Duration> for UnixTimestamp {
    fn add_assign(&mut self, rhs: Duration) {
        self.millis += rhs.as_millis();
    }
}

impl Add<Duration> for UnixTimestamp {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Self {
            millis: self.millis + rhs.as_millis(),
        }
    }
}

impl Sub<Duration> for UnixTimestamp {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        Self {
            millis: self.millis - rhs.as_millis(),
        }
    }
}

impl Sub<UnixTimestamp> for UnixTimestamp {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        let diff = self.millis - rhs.millis;
        Duration::from_millis(diff as u64)
    }
}
