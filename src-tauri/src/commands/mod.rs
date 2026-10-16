//! Thin command surface (spec §9.2); business logic lives behind typed APIs.

pub mod auth;
pub mod commits;
pub mod history;
pub mod promotion;
pub mod registry;
pub mod system;
pub mod workflows;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::FixtureWorld;
use crate::AppState;

pub(crate) fn read_world<T>(
    state: &AppState,
    f: impl FnOnce(&FixtureWorld) -> AppResult<T>,
) -> AppResult<T> {
    let world = state.world.lock().map_err(|_| {
        AppError::new(ErrorCode::Internal, "The fixture world lock is poisoned.")
            .with_remedy("Restart the application; report this if it recurs.")
    })?;
    f(&world)
}

pub(crate) fn write_world<T>(
    state: &AppState,
    f: impl FnOnce(&mut FixtureWorld) -> AppResult<T>,
) -> AppResult<T> {
    let mut world = state.world.lock().map_err(|_| {
        AppError::new(ErrorCode::Internal, "The fixture world lock is poisoned.")
            .with_remedy("Restart the application; report this if it recurs.")
    })?;
    f(&mut world)
}
