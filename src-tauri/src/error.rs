//! Typed errors for the Tauri command layer (spec §9.2).
//! Serializes to the `ApiErrorPayload` shape in `src/api/invoke.ts`
//! (`code` + `message` + `remedy`).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthenticated,
    MissingPrerequisite,
    InsufficientScope,
    RepositoryInaccessible,
    InvalidRegistry,
    BranchNotPermitted,
    SourceCommitNotEligible,
    DuplicateIntegration,
    DuplicateRegistration,
    RateLimited,
    Offline,
    WorkflowDispatchRejected,
    PatchConflict,
    DependencyConflict,
    CiFailure,
    NotFound,
    InvalidRequest,
    Internal,
}

#[derive(Debug, Clone)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub remedy: Option<String>,
    pub details: Option<String>,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            remedy: None,
            details: None,
            retryable: false,
        }
    }

    pub fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn invalid_registry(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRegistry, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ApiErrorPayload", 5)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("message", &self.message)?;
        state.serialize_field("remedy", &self.remedy)?;
        state.serialize_field("details", &self.details)?;
        state.serialize_field("retryable", &self.retryable)?;
        state.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;
