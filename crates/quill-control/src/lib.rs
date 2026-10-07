pub mod bootstrap;
pub mod clock;
pub mod error;
pub mod identity;
pub mod password;
pub mod repo;
pub mod secret;
pub mod service;
pub mod user;

pub use bootstrap::{
    ensure_password_user, ensure_token_user, PasswordProvision, Provision, TOKEN_ONLY_ALGO,
};
pub use clock::{Clock, ManualClock, SystemClock};
pub use error::ControlError;
pub use identity::{derive_user_id, parse_token_subject, TokenSubject};
pub use password::{
    constant_time_eq, hmac_sha256, pbkdf2_sha256, validate_password, PasswordDigest,
    PasswordHasher, Pbkdf2Params, DK_BYTES, MAX_PASSWORD_LEN, MIN_PASSWORD_LEN, PBKDF2_ALGO_PREFIX,
    SALT_BYTES,
};
pub use secret::{
    entropy_unavailable_message, invite_code_digest, new_invite_code, new_session_token,
    session_token_digest, OsEntropySource, SecretSource, INVITE_CODE_BYTES, INVITE_CODE_HEX_LEN,
    TOKEN_BYTES, TOKEN_HEX_LEN,
};

pub use service::{
    AuthPolicy, AuthSession, Authenticated, ControlPlane, InviteSummary, IssuedInvite,
    LogoutOutcome, RegistrationRequest, DEFAULT_LOCALE, MAX_INVITE_USES,
};
pub use user::{
    normalize_username, validate_display_name, validate_username, UserProfile, UserRole,
    UserStatus, MAX_DISPLAY_NAME_LEN, MAX_USERNAME_LEN, MIN_USERNAME_LEN,
};

pub use quill_domain::{SessionId, UserId, UuidBytes};
