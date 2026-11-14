//! acorn's token types (`tokentype.js`) and tokenizer contexts
//! (`tokencontext.js`).

/// A token type. The keyword types carry acorn's leading underscore as
/// a `Kw` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    Num,
    Regexp,
    String,
    Name,
    PrivateId,
    Eof,
    BracketL,
    BracketR,
    BraceL,
    BraceR,
    ParenL,
    ParenR,
    Comma,
    Semi,
    Colon,
    Dot,
    Question,
    QuestionDot,
    Arrow,
    Template,
    InvalidTemplate,
    Ellipsis,
    BackQuote,
    DollarBraceL,
    Eq,
    Assign,
    IncDec,
    Prefix,
    LogicalOr,
    LogicalAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    Equality,
    Relational,
    BitShift,
    PlusMin,
    Modulo,
    Star,
    Slash,
    StarStar,
    Coalesce,
    KwBreak,
    KwCase,
    KwCatch,
    KwContinue,
    KwDebugger,
    KwDefault,
    KwDo,
    KwElse,
    KwFinally,
    KwFor,
    KwFunction,
    KwIf,
    KwReturn,
    KwSwitch,
    KwThrow,
    KwTry,
    KwVar,
    KwConst,
    KwWhile,
    KwWith,
    KwNew,
    KwThis,
    KwSuper,
    KwClass,
    KwExtends,
    KwExport,
    KwImport,
    KwNull,
    KwTrue,
    KwFalse,
    KwIn,
    KwInstanceof,
    KwTypeof,
    KwVoid,
    KwDelete,
    /// `@`, before a decorator. TypeScript only.
    At,
    // Words acorn-typescript gives a token type of their own. They are
    // names everywhere a name may stand, but they are not `Name`: what
    // acorn itself asks of a `Name` token does not hold for them.
    TsAssert,
    TsAsserts,
    TsGlobal,
    TsKeyof,
    TsReadonly,
    TsUnique,
    TsAbstract,
    TsDeclare,
    TsEnum,
    TsModule,
    TsNamespace,
    TsInterface,
    TsType,
}

impl Tok {
    /// The token type acorn-typescript gives the word, if it has one.
    pub(crate) fn of_ts_word(word: &str) -> Option<Tok> {
        Some(match word {
            "assert" => Tok::TsAssert,
            "asserts" => Tok::TsAsserts,
            "global" => Tok::TsGlobal,
            "keyof" => Tok::TsKeyof,
            "readonly" => Tok::TsReadonly,
            "unique" => Tok::TsUnique,
            "abstract" => Tok::TsAbstract,
            "declare" => Tok::TsDeclare,
            "enum" => Tok::TsEnum,
            "module" => Tok::TsModule,
            "namespace" => Tok::TsNamespace,
            "interface" => Tok::TsInterface,
            "type" => Tok::TsType,
            _ => return None,
        })
    }

    /// One of acorn-typescript's own word types.
    pub(crate) fn is_ts_word(self) -> bool {
        matches!(
            self,
            Tok::TsAssert
                | Tok::TsAsserts
                | Tok::TsGlobal
                | Tok::TsKeyof
                | Tok::TsReadonly
                | Tok::TsUnique
                | Tok::TsAbstract
                | Tok::TsDeclare
                | Tok::TsEnum
                | Tok::TsModule
                | Tok::TsNamespace
                | Tok::TsInterface
                | Tok::TsType
        )
    }

    /// `tokenIsIdentifier`: a name, in TypeScript's eyes.
    pub(crate) fn is_identifier(self) -> bool {
        self == Tok::Name || self.is_ts_word()
    }

    /// `tokenIsKeywordOrIdentifier`.
    pub(crate) fn is_keyword_or_identifier(self) -> bool {
        self.is_identifier() || self.keyword().is_some()
    }

    /// `tokenIsLiteralPropertyName`.
    pub(crate) fn is_literal_property_name(self) -> bool {
        self.is_keyword_or_identifier() || self == Tok::String || self == Tok::Num
    }

    /// `tokenIsTSDeclarationStart`.
    pub(crate) fn is_ts_declaration_start(self) -> bool {
        matches!(
            self,
            Tok::TsAbstract
                | Tok::TsDeclare
                | Tok::TsEnum
                | Tok::TsModule
                | Tok::TsNamespace
                | Tok::TsInterface
                | Tok::TsType
        )
    }

    /// The type of the keyword `word`.
    pub(crate) fn of_keyword(word: &str) -> Option<Tok> {
        Some(match word {
            "break" => Tok::KwBreak,
            "case" => Tok::KwCase,
            "catch" => Tok::KwCatch,
            "continue" => Tok::KwContinue,
            "debugger" => Tok::KwDebugger,
            "default" => Tok::KwDefault,
            "do" => Tok::KwDo,
            "else" => Tok::KwElse,
            "finally" => Tok::KwFinally,
            "for" => Tok::KwFor,
            "function" => Tok::KwFunction,
            "if" => Tok::KwIf,
            "return" => Tok::KwReturn,
            "switch" => Tok::KwSwitch,
            "throw" => Tok::KwThrow,
            "try" => Tok::KwTry,
            "var" => Tok::KwVar,
            "const" => Tok::KwConst,
            "while" => Tok::KwWhile,
            "with" => Tok::KwWith,
            "new" => Tok::KwNew,
            "this" => Tok::KwThis,
            "super" => Tok::KwSuper,
            "class" => Tok::KwClass,
            "extends" => Tok::KwExtends,
            "export" => Tok::KwExport,
            "import" => Tok::KwImport,
            "null" => Tok::KwNull,
            "true" => Tok::KwTrue,
            "false" => Tok::KwFalse,
            "in" => Tok::KwIn,
            "instanceof" => Tok::KwInstanceof,
            "typeof" => Tok::KwTypeof,
            "void" => Tok::KwVoid,
            "delete" => Tok::KwDelete,
            _ => return None,
        })
    }

    /// The keyword this type is, if it is one.
    pub(crate) fn keyword(self) -> Option<&'static str> {
        Some(match self {
            Tok::KwBreak => "break",
            Tok::KwCase => "case",
            Tok::KwCatch => "catch",
            Tok::KwContinue => "continue",
            Tok::KwDebugger => "debugger",
            Tok::KwDefault => "default",
            Tok::KwDo => "do",
            Tok::KwElse => "else",
            Tok::KwFinally => "finally",
            Tok::KwFor => "for",
            Tok::KwFunction => "function",
            Tok::KwIf => "if",
            Tok::KwReturn => "return",
            Tok::KwSwitch => "switch",
            Tok::KwThrow => "throw",
            Tok::KwTry => "try",
            Tok::KwVar => "var",
            Tok::KwConst => "const",
            Tok::KwWhile => "while",
            Tok::KwWith => "with",
            Tok::KwNew => "new",
            Tok::KwThis => "this",
            Tok::KwSuper => "super",
            Tok::KwClass => "class",
            Tok::KwExtends => "extends",
            Tok::KwExport => "export",
            Tok::KwImport => "import",
            Tok::KwNull => "null",
            Tok::KwTrue => "true",
            Tok::KwFalse => "false",
            Tok::KwIn => "in",
            Tok::KwInstanceof => "instanceof",
            Tok::KwTypeof => "typeof",
            Tok::KwVoid => "void",
            Tok::KwDelete => "delete",
            _ => return None,
        })
    }

    /// An expression may follow: a `/` after it starts a regular
    /// expression.
    pub(crate) fn before_expr(self) -> bool {
        matches!(
            self,
            Tok::BracketL
                | Tok::BraceL
                | Tok::ParenL
                | Tok::Comma
                | Tok::Semi
                | Tok::Colon
                | Tok::Question
                | Tok::Arrow
                | Tok::Ellipsis
                | Tok::DollarBraceL
                | Tok::Eq
                | Tok::Assign
                | Tok::Prefix
                | Tok::LogicalOr
                | Tok::LogicalAnd
                | Tok::BitwiseOr
                | Tok::BitwiseXor
                | Tok::BitwiseAnd
                | Tok::Equality
                | Tok::Relational
                | Tok::BitShift
                | Tok::PlusMin
                | Tok::Modulo
                | Tok::Star
                | Tok::Slash
                | Tok::StarStar
                | Tok::Coalesce
                | Tok::KwCase
                | Tok::KwDefault
                | Tok::KwDo
                | Tok::KwElse
                | Tok::KwReturn
                | Tok::KwThrow
                | Tok::KwNew
                | Tok::KwExtends
                | Tok::KwIn
                | Tok::KwInstanceof
                | Tok::KwTypeof
                | Tok::KwVoid
                | Tok::KwDelete
        )
    }

    pub(crate) fn starts_expr(self) -> bool {
        matches!(
            self,
            Tok::Num
                | Tok::Regexp
                | Tok::String
                | Tok::Name
                | Tok::PrivateId
                | Tok::BracketL
                | Tok::BraceL
                | Tok::ParenL
                | Tok::BackQuote
                | Tok::DollarBraceL
                | Tok::IncDec
                | Tok::Prefix
                | Tok::PlusMin
                | Tok::KwFunction
                | Tok::KwNew
                | Tok::KwThis
                | Tok::KwSuper
                | Tok::KwClass
                | Tok::KwImport
                | Tok::KwNull
                | Tok::KwTrue
                | Tok::KwFalse
                | Tok::KwTypeof
                | Tok::KwVoid
                | Tok::KwDelete
        ) || self.is_ts_word()
    }

    pub(crate) fn is_loop(self) -> bool {
        matches!(self, Tok::KwDo | Tok::KwFor | Tok::KwWhile)
    }

    pub(crate) fn is_assign(self) -> bool {
        matches!(self, Tok::Eq | Tok::Assign)
    }

    pub(crate) fn prefix(self) -> bool {
        matches!(
            self,
            Tok::IncDec | Tok::Prefix | Tok::PlusMin | Tok::KwTypeof | Tok::KwVoid | Tok::KwDelete
        )
    }

    pub(crate) fn postfix(self) -> bool {
        self == Tok::IncDec
    }

    /// The precedence of the binary operator this type is.
    pub(crate) fn binop(self) -> Option<i32> {
        Some(match self {
            Tok::LogicalOr | Tok::Coalesce => 1,
            Tok::LogicalAnd => 2,
            Tok::BitwiseOr => 3,
            Tok::BitwiseXor => 4,
            Tok::BitwiseAnd => 5,
            Tok::Equality => 6,
            Tok::Relational | Tok::KwIn | Tok::KwInstanceof => 7,
            Tok::BitShift => 8,
            Tok::PlusMin => 9,
            Tok::Modulo | Tok::Star | Tok::Slash => 10,
            _ => return None,
        })
    }
}

/// What the tokenizer is inside of. It decides whether a `{` opens a
/// block or an object and whether a `/` divides or starts a regular
/// expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    BStat,
    BExpr,
    BTmpl,
    PStat,
    PExpr,
    QTmpl,
    FStat,
    FExpr,
    FExprGen,
    FGen,
}

impl Ctx {
    pub(crate) fn is_function(self) -> bool {
        matches!(self, Ctx::FStat | Ctx::FExpr | Ctx::FExprGen | Ctx::FGen)
    }

    pub(crate) fn is_expr(self) -> bool {
        matches!(
            self,
            Ctx::BExpr | Ctx::PExpr | Ctx::QTmpl | Ctx::FExpr | Ctx::FExprGen
        )
    }

    pub(crate) fn generator(self) -> bool {
        matches!(self, Ctx::FExprGen | Ctx::FGen)
    }
}
