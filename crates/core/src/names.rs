//! The names the records and the contract use: kinds, urgencies, states,
//! authors and event types.

use alloc::format;
use core::fmt;
use core::str::FromStr;

use crate::error::{Category, Error};

/// Declares an enum of names, each spelt in the records as its string.
macro_rules! names {
    (
        $(#[$meta:meta])*
        $name:ident, $what:literal, $category:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $text:literal,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every value, in the order the contract lists them.
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            /// The value as the records spell it.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }
        }

        impl FromStr for $name {
            type Err = Error;

            #[doc = concat!(
                "Reads ", $what, ", matched exactly. Any other string is an error of category [`",
                stringify!($category), "`](Category::", stringify!($category), ")."
            )]
            fn from_str(s: &str) -> Result<$name, Error> {
                match s {
                    $($text => Ok($name::$variant),)+
                    _ => Err(Error::new(
                        Category::$category,
                        format!(concat!("{:?} is not ", $what), s),
                    )),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

names! {
    /// What sort of answer a case asks for, or that it is a message from the
    /// human.
    Kind, "a kind", Unknown {
        /// Choose one of the options, or other with a note.
        Decision = "decision",
        /// A verdict on each row: approve, hold or reject.
        Approval = "approval",
        /// Accept, or ask for changes with a note.
        Signoff = "signoff",
        /// The agent is stuck and needs guidance. Only a stuck case parks.
        Stuck = "stuck",
        /// A question with a text answer.
        Question = "question",
        /// For information: the answer acknowledges it.
        Fyi = "fyi",
        /// A message from the human to an agent: the one kind the human
        /// opens. An agent picks it up and closes it with the outcome. It
        /// takes no answer.
        Message = "message",
        /// A document showing the human finished work: accept it, or send
        /// it back with a reason, and file any of what it left out.
        Briefing = "briefing",
    }
}

names! {
    /// How soon a case needs its answer. The order is the inbox's, most
    /// urgent first.
    Urgency, "an urgency", Unknown {
        /// Work stops until it is answered.
        Blocking = "blocking",
        /// Today.
        Today = "today",
        /// Whenever.
        Whenever = "whenever",
    }
}

names! {
    /// Where a case is. The fold works it out from the events; it is never
    /// stored.
    State, "a state", Unknown {
        /// Waiting for an answer.
        Open = "open",
        /// Answered, and waiting for the agent to pick the answer up.
        Answered = "answered",
        /// The agent has the answer, or has taken the message.
        PickedUp = "pickedup",
        /// Closed with its outcome.
        Closed = "closed",
        /// Withdrawn: taken back before it was answered, or ended by the
        /// human while an agent had it.
        Withdrawn = "withdrawn",
        /// A stuck case the human has put aside.
        Parked = "parked",
        /// A message the human sent, waiting for an agent to pick it up.
        Sent = "sent",
    }
}

names! {
    /// Who writes an event. The author is the role; an event's
    /// [`Actor`](crate::Actor) says which agent or human it was.
    Author, "an author", Author {
        /// The agent that opened the case, or that acts on a message.
        Agent = "agent",
        /// The human who answers it, or who sent the message.
        Human = "human",
    }
}

names! {
    /// What an event does.
    EventType, "an event", Unknown {
        /// Opens the case: the agent's, or the human's for a message.
        Open = "open",
        /// Changes an open case, or a message that is sent: by the side
        /// that opened it.
        Amend = "amend",
        /// Answers an open case.
        Answer = "answer",
        /// The agent has read the answer, or takes a message that is sent.
        Pickup = "pickup",
        /// Adds to the thread, and asks for a new answer when the case has one.
        Note = "note",
        /// Closes a picked-up case with its outcome.
        Close = "close",
        /// Withdraws an open case, or, by the human, a sent message or an
        /// answered or picked-up case.
        Withdraw = "withdraw",
        /// Puts an open stuck case aside.
        Park = "park",
        /// Opens a parked case again.
        Resume = "resume",
    }
}

impl Kind {
    /// The side that opens a case of this kind, and so writes its open and
    /// its amends: the human a message, and the agent every other kind. A
    /// case waits first on the other side.
    #[must_use]
    pub fn opener(self) -> Author {
        match self {
            Kind::Message => Author::Human,
            Kind::Decision
            | Kind::Approval
            | Kind::Signoff
            | Kind::Stuck
            | Kind::Question
            | Kind::Fyi
            | Kind::Briefing => Author::Agent,
        }
    }
}

impl EventType {
    /// Whether `author` may write the event on some case. The agent writes
    /// `pickup`, `note` and `close`, and the human `answer`, `park` and
    /// `resume`. Either writes `withdraw`, and either writes `open` and
    /// `amend`, which belong to the side that opens the case's kind, its
    /// [`opener`](Kind::opener): the human's on a message, and the agent's
    /// on every other kind. That is checked once the kind is known.
    #[must_use]
    pub fn written_by(self, author: Author) -> bool {
        match self {
            EventType::Pickup | EventType::Note | EventType::Close => author == Author::Agent,
            EventType::Answer | EventType::Park | EventType::Resume => author == Author::Human,
            EventType::Open | EventType::Amend | EventType::Withdraw => true,
        }
    }
}
