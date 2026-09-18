//! The legacy event log's names for the session event properties, which
//! live in `crate::session_event`.

pub use crate::session_event::{
    SessionAssistantDeltaProps as RunSessionAssistantDeltaProps,
    SessionAssistantMessageProps as RunSessionAssistantMessageProps,
    SessionCreatedProps as RunSessionCreatedProps,
    SessionToolCallCompletedProps as RunSessionToolCallCompletedProps,
    SessionToolCallStartedProps as RunSessionToolCallStartedProps,
    SessionTurnFailedCode as RunSessionTurnFailedCode,
    SessionTurnFailedProps as RunSessionTurnFailedProps,
    SessionTurnInterruptedProps as RunSessionTurnInterruptedProps,
    SessionTurnStartedProps as RunSessionTurnStartedProps,
    SessionTurnSucceededProps as RunSessionTurnSucceededProps,
    SessionUserMessageProps as RunSessionUserMessageProps,
};
