// This file is part of the shakmaty library.
// Copyright (C) 2017 Niklas Fiekas <niklas.fiekas@backscattering.de>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <http://www.gnu.org/licenses/>.

use attacks;
use board::Board;
use bitboard;
use bitboard::Bitboard;
use square;
use square::Square;
use types::{Color, White, Black, Role, Piece, Move, Pockets, RemainingChecks};
use setup;
use setup::Setup;
use util;

use option_filter::OptionFilterExt;
use arrayvec::ArrayVec;

use std::fmt;
use std::error::Error;

/// Outcome of a game.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Outcome {
    Decisive { winner: Color },
    Draw
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match *self {
            Outcome::Decisive { winner: White } => "1-0",
            Outcome::Decisive { winner: Black } => "0-1",
            Outcome::Draw => "1/2-1/2",
        })
    }
}

/// Reasons for a `Setup` not beeing a legal `Position`.
#[derive(Debug)]
pub enum PositionError {
    Empty,
    NoKing { color: Color },
    TooManyPawns,
    TooManyPieces,
    TooManyKings,
    PawnsOnBackrank,
    BadCastlingRights,
    InvalidEpSquare,
    OppositeCheck,
    ThreeCheckOver,
    RacingKingsCheck,
    RacingKingsOver,
    RacingKingsMaterial,
}

impl PositionError {
    fn desc(&self) -> &str {
        match *self {
            PositionError::Empty => "empty board is not legal",
            PositionError::NoKing { color: White } => "white king missing",
            PositionError::NoKing { color: Black } => "black king missing",
            PositionError::TooManyPawns => "too many pawns",
            PositionError::TooManyPieces => "too many pieces",
            PositionError::TooManyKings => "too many kings",
            PositionError::PawnsOnBackrank => "pawns on backrank",
            PositionError::BadCastlingRights => "bad castling rights",
            PositionError::InvalidEpSquare => "invalid en passant square",
            PositionError::OppositeCheck => "opponent is in check",
            PositionError::ThreeCheckOver => "no remaining checks",
            PositionError::RacingKingsCheck => "check in racing kings",
            PositionError::RacingKingsOver => "race should have ended before",
            PositionError::RacingKingsMaterial => "illegal racing kings material",
        }
    }
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.desc().fmt(f)
    }
}

impl Error for PositionError {
    fn description(&self) -> &str { self.desc() }
}

/// Error in case of illegal moves.
#[derive(Debug)]
pub struct IllegalMove { }

impl fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "illegal move".fmt(f)
    }
}

impl Error for IllegalMove {
    fn description(&self) -> &str { "illegal move" }
}

/// A stack-allocated container to hold legal moves.
pub type MoveList = ArrayVec<[Move; 512]>;

/// A legal chess or chess variant position. See `Chess` and
/// `shakmaty::variants` for concrete implementations.
pub trait Position: Setup + Default + Clone {
    /// Whether or not promoted pieces are special in the respective chess
    /// variant. For example in Crazyhouse a promoted queen should be marked
    /// as `Q~` in FENs and will become a pawn when captured.
    const TRACK_PROMOTED: bool;

    /// Wether pawns can be promoted to kings in this variant.
    const KING_PROMOTIONS: bool;

    /// Validates a `Setup` and constructs a position.
    fn from_setup<S: Setup>(setup: &S) -> Result<Self, PositionError>;

    /// Attacks that a king on `square` would have to deal with.
    fn king_attackers(&self, square: Square, attacker: Color, occupied: Bitboard) -> Bitboard {
        self.board().attacks_to(square, attacker, occupied)
    }

    /// Tests the rare case where moving the rook to the other side during
    /// castling would uncover a rank attack.
    fn castling_uncovers_rank_attack(&self, rook: Square, king_to: Square) -> bool {
        castling_uncovers_rank_attack(self, rook, king_to)
    }

    /// Bitboard of pieces giving check.
    fn checkers(&self) -> Bitboard {
        self.our(Role::King).first()
            .map_or(Bitboard(0), |king| self.king_attackers(king, !self.turn(), self.board().occupied()))
    }

    /// Generates legal moves.
    fn legal_moves(&self, moves: &mut MoveList);

    /// Generates a subset of legal moves.
    fn san_candidates(&self, role: Role, to: Square, moves: &mut MoveList) {
        self.legal_moves(moves);
        filter_san_candidates(role, to, moves);
    }

    /// Tests a move for legality.
    fn is_legal(&self, m: &Move) -> bool {
        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals.contains(m)
    }

    /// Tests if a move zeros the halfmove clock.
    fn is_zeroing(&self, m: &Move) -> bool {
        match *m {
            Move::Normal { capture: Some(_), .. } |
                Move::Normal { role: Role::Pawn, .. } |
                Move::EnPassant { .. } => true,
            _ => false
        }
    }

    /// Checks if the game is over due to a special variant end condition.
    ///
    /// Note that for example stalemate is not considered a variant-specific
    /// end condition (`is_variant_end()` will return `false`), but it can have
    /// a special `variant_outcome()` in suicide chess.
    fn is_variant_end(&self) -> bool;

    /// Tests for checkmate.
    fn is_checkmate(&self) -> bool {
        if self.checkers().is_empty() {
            return false;
        }

        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals.is_empty()
    }

    /// Tests for stalemate.
    fn is_stalemate(&self) -> bool {
        if !self.checkers().is_empty() || self.is_variant_end() {
            false
        } else {
            let mut legals = MoveList::new();
            self.legal_moves(&mut legals);
            legals.is_empty()
        }
    }

    /// Tests for insufficient winning material.
    fn is_insufficient_material(&self) -> bool;

    /// Tests if the game is over due to checkmate, stalemate, insufficient
    /// material or variant end.
    fn is_game_over(&self) -> bool {
        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals.is_empty() || self.is_insufficient_material()
    }

    /// Tests special variant winning, losing and drawing conditions.
    fn variant_outcome(&self) -> Option<Outcome>;

    /// The outcome of the game, or `None` if the game is not over.
    fn outcome(&self) -> Option<Outcome> {
        self.variant_outcome().or_else(|| {
            if self.is_checkmate() {
                Some(Outcome::Decisive { winner: !self.turn() }) // checkmate
            } else if self.is_stalemate() || self.is_insufficient_material() {
                Some(Outcome::Draw)
            } else {
                None
            }
        })
    }

    /// Validates and plays a move. Accepts only legal moves and
    /// safe null moves.
    fn play(self, m: &Move) -> Result<Self, IllegalMove> {
        if self.is_legal(m) {
            Ok(self.play_unchecked(m))
        } else {
            Err(IllegalMove {})
        }
    }

    /// Plays a move. It is the callers responsibility to ensure the move is
    /// legal.
    ///
    /// # Panics
    ///
    /// Illegal moves can corrupt the state of the position and may
    /// (or may not) panic or cause panics on future calls.
    fn play_unchecked(self, m: &Move) -> Self;
}

/// A standard Chess position.
#[derive(Clone, Debug)]
pub struct Chess {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for Chess {
    fn default() -> Chess {
        Chess {
            board: Board::default(),
            turn: White,
            castling_rights: bitboard::CORNERS,
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for Chess {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for Chess {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> Chess {
        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);
        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Chess, PositionError> {
        let pos = Chess {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        validate_basic(&pos)
            .or_else(|| validate_kings(&pos))
            .map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        gen_standard(self, self.ep_square, moves);
    }

    fn san_candidates(&self, role: Role, to: Square, moves: &mut MoveList) {
        let our_king = self.our(Role::King).first();
        let king = our_king.expect("got a king");

        let checkers = self.checkers();

        if checkers.is_empty() {
            let target = !self.us() & to;
            let blockers = slider_blockers(self.board(), self.them(), king);
            match role {
                Role::Pawn => {
                    gen_en_passant(self.board(), self.turn(), self.ep_square, our_king, moves);
                    gen_pawn_moves(self, target, moves, |from, to| {
                        !blockers.contains(from) || attacks::aligned(from, to, king)
                    });
                },
                Role::Knight =>
                    KnightTag::gen_safe_moves(self, target, blockers, moves),
                Role::Bishop =>
                    BishopTag::gen_safe_moves(self, target, king, blockers, moves),
                Role::Rook =>
                    RookTag::gen_safe_moves(self, target, king, blockers, moves),
                Role::Queen =>
                    QueenTag::gen_safe_moves(self, target, king, blockers, moves),
                Role::King => {
                    gen_safe_king(self, target, moves);
                    gen_castling_moves(self, king, moves);
                },
            }
        } else {
            gen_en_passant(self.board(), self.turn(), self.ep_square, our_king, moves);
            evasions(self, king, checkers, moves);
            filter_san_candidates(role, to, moves);
        }
    }

    fn is_insufficient_material(&self) -> bool {
        if self.board().pawns().any() || self.board().rooks_and_queens().any() {
            return false;
        }

        if self.board().occupied().count() < 3 {
            return true; // single knight or bishop
        }

        if self.board().knights().any() {
            return false; // more than a single knight
        }

        // all bishops on the same color
        if (self.board().bishops() & bitboard::DARK_SQUARES).is_empty() {
            return true;
        }
        if (self.board().bishops() & bitboard::LIGHT_SQUARES).is_empty() {
            return true;
        }

        false
    }

    fn is_variant_end(&self) -> bool { false }
    fn variant_outcome(&self) -> Option<Outcome> { None }
}

/// A Crazyhouse position.
#[derive(Clone, Debug)]
pub struct Crazyhouse {
    board: Board,
    pockets: Pockets,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Setup for Crazyhouse {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { Some(&self.pockets) }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Default for Crazyhouse {
    fn default() -> Crazyhouse {
        Crazyhouse {
            board: Board::default(),
            pockets: Pockets::default(),
            turn: White,
            castling_rights: bitboard::CORNERS,
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Crazyhouse {
    fn legal_put_squares(&self) -> Bitboard {
        let checkers = self.checkers();

        if checkers.is_empty() {
            !self.board().occupied()
        } else if let Some(checker) = checkers.single_square() {
            let king = self.our(Role::King).first().expect("has a king");
            return attacks::between(checker, king)
        } else {
            Bitboard(0)
        }
    }
}

impl Position for Crazyhouse {
    const TRACK_PROMOTED: bool = true;
    const KING_PROMOTIONS: bool = false;

    fn is_zeroing(&self, _: &Move) -> bool {
        false
    }

    fn play_unchecked(mut self, m: &Move) -> Crazyhouse {
        let turn = self.turn();
        let mut fake_halfmove_clock = 0;

        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut fake_halfmove_clock,
                &mut self.fullmoves, m);

        match *m {
            Move::Normal { capture: Some(role), .. } =>
                self.pockets.add(role.of(turn)),
            Move::EnPassant { .. } =>
                self.pockets.add(turn.pawn()),
            Move::Put { role, .. } =>
                self.pockets.remove(&role.of(turn)),
            _ => ()
        }

        self.halfmove_clock = self.halfmove_clock.saturating_add(1);
        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Self, PositionError> {
        let pos = Crazyhouse {
            board: setup.board().clone(),
            pockets: setup.pockets().map_or(Pockets::default(), |p| p.clone()),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        validate_basic(&pos)
            .or_else(|| validate_kings(&pos))
            .map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        gen_standard(self, self.ep_square, moves);

        for to in self.legal_put_squares() {
            for role in &[Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
                if self.pockets.by_piece(&role.of(self.turn())) > 0 {
                    moves.push(Move::Put { role: *role, to });
                }
            }

            if 0 < to.rank() && to.rank() < 7 && self.pockets.by_color(self.turn()).pawns > 0 {
                moves.push(Move::Put { role: Role::Pawn, to });
            }
        }
    }

    fn is_insufficient_material(&self) -> bool {
        false
    }

    fn is_variant_end(&self) -> bool { false }
    fn variant_outcome(&self) -> Option<Outcome> { None }
}

/// A King of the Hill position.
#[derive(Clone, Debug)]
pub struct KingOfTheHill {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for KingOfTheHill {
    fn default() -> KingOfTheHill {
        KingOfTheHill {
            board: Board::default(),
            turn: White,
            castling_rights: bitboard::CORNERS,
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for KingOfTheHill {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for KingOfTheHill {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> KingOfTheHill {
        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);
        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<KingOfTheHill, PositionError> {
        let pos = KingOfTheHill {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        validate_basic(&pos)
            .or_else(|| validate_kings(&pos))
            .map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        if !self.is_variant_end() {
            gen_standard(self, self.ep_square, moves);
        }
    }

    fn is_insufficient_material(&self) -> bool {
        false
    }

    fn is_variant_end(&self) -> bool {
        (self.board().kings() & bitboard::HILL).any()
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        for color in &[White, Black] {
            if (self.board().by_piece(&color.king()) & bitboard::HILL).any() {
                return Some(Outcome::Decisive { winner: *color })
            }
        }
        None
    }
}

/// A Giveaway position.
#[derive(Clone, Debug)]
pub struct Giveaway {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for Giveaway {
    fn default() -> Giveaway {
        Giveaway {
            board: Board::default(),
            turn: White,
            castling_rights: Bitboard(0),
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for Giveaway {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for Giveaway {
    // prevent promoted kings from beeing able to castle
    const TRACK_PROMOTED: bool = true;
    const KING_PROMOTIONS: bool = true;

    fn play_unchecked(mut self, m: &Move) -> Giveaway {
        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);
        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Giveaway, PositionError> {
        let pos = Giveaway {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        validate_basic(&pos).map_or(Ok(pos), Err)
    }

    fn is_variant_end(&self) -> bool {
        self.board().white().is_empty() || self.board().black().is_empty()
    }

    fn king_attackers(&self, _square: Square, _attacker: Color, _occupied: Bitboard) -> Bitboard {
        Bitboard(0)
    }

    fn castling_uncovers_rank_attack(&self, _rook: Square, _king_to: Square) -> bool {
        false
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        let them = self.them();

        gen_en_passant(self.board(), self.turn(), self.ep_square, None, moves);
        gen_non_king(self, them, moves);
        KingTag::gen_moves(self, them, moves);

        if moves.is_empty() {
            gen_non_king(self, !self.board().occupied(), moves);
            KingTag::gen_moves(self, !self.board().occupied(), moves);
            self.board().king_of(self.turn()).map(|king| gen_castling_moves(self, king, moves));
        }
    }

    fn is_insufficient_material(&self) -> bool {
        if self.board().knights().any() || self.board().rooks_and_queens().any() || self.board().kings().any() {
            return false;
        }

        if self.board().pawns().any() {
            // TODO: Could detect blocked pawns.
            return false;
        }

        // Detect bishops and pawns of each side on distinct color complexes.
        if (self.board().white() & bitboard::DARK_SQUARES).is_empty() {
            (self.board().black() & bitboard::LIGHT_SQUARES).is_empty()
        } else if (self.board().white() & bitboard::LIGHT_SQUARES).is_empty() {
            (self.board().black() & bitboard::DARK_SQUARES).is_empty()
        } else {
            false
        }
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        if self.us().is_empty() || self.is_stalemate() {
            Some(Outcome::Decisive { winner: self.turn() })
        } else {
            None
        }
    }
}

/// A Three Check position.
#[derive(Clone, Debug)]
pub struct ThreeCheck {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    remaining_checks: RemainingChecks,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for ThreeCheck {
    fn default() -> ThreeCheck {
        ThreeCheck {
            board: Board::default(),
            turn: White,
            castling_rights: bitboard::CORNERS,
            ep_square: None,
            remaining_checks: RemainingChecks::default(),
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for ThreeCheck {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { Some(&self.remaining_checks) }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for ThreeCheck {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> ThreeCheck {
        let turn = self.turn();

        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);

        if self.checkers().any() {
            self.remaining_checks.subtract(turn);
        }

        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<ThreeCheck, PositionError> {
        let checks = setup.remaining_checks()
                          .map_or(RemainingChecks::default(), |c| c.clone());

        if checks.white == 0 && checks.black == 0 {
            return Err(PositionError::ThreeCheckOver);
        }

        let pos = ThreeCheck {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            remaining_checks: checks,
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        validate_basic(&pos)
            .or_else(|| validate_kings(&pos))
            .map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        if !self.is_variant_end() {
            gen_standard(self, self.ep_square, moves);
        }
    }

    fn is_insufficient_material(&self) -> bool {
        self.board().occupied() == self.board().kings()
    }

    fn is_variant_end(&self) -> bool {
        self.remaining_checks.white == 0 || self.remaining_checks.black == 0
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        if self.remaining_checks.white == 0 {
            Some(Outcome::Decisive { winner: White })
        } else if self.remaining_checks.black == 0 {
            Some(Outcome::Decisive { winner: Black })
        } else {
            None
        }
    }
}

/// A Horde position.
#[derive(Clone, Debug)]
pub struct Horde {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for Horde {
    fn default() -> Horde {
        Horde {
            board: Board::horde(),
            turn: White,
            castling_rights: Bitboard::from_square(square::A8).with(square::H8),
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for Horde {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for Horde {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> Horde {
        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);
        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Horde, PositionError> {
        let pos = Horde {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        if pos.board().occupied().is_empty() {
            return Err(PositionError::Empty);
        }

        if pos.board().by_piece(&Black.king()).is_empty() {
            return Err(PositionError::NoKing { color: Black });
        }
        if pos.board().kings().count() > 1 {
            return Err(PositionError::TooManyKings);
        }

        if pos.board().black().count() > 16 {
            return Err(PositionError::TooManyPieces);
        }
        if pos.board().white().count() > 36 {
            return Err(PositionError::TooManyPieces);
        }

        if pos.board().by_piece(&Black.pawn()).count() > 8 {
            return Err(PositionError::TooManyPawns);
        }
        if pos.board().by_piece(&White.pawn()).count() > 36 {
            return Err(PositionError::TooManyPawns);
        }

        for color in &[White, Black] {
            if (pos.board().by_piece(&color.pawn()) & Bitboard::relative_rank(*color, 7)).any() {
                return Err(PositionError::PawnsOnBackrank);
            }
        }

        if pos.castling_rights() != setup::clean_castling_rights(&pos, false) & Bitboard::rank(7) {
            return Err(PositionError::BadCastlingRights);
        }

        validate_ep(&pos).map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        gen_standard(self, self.ep_square, moves);
    }

    fn is_insufficient_material(&self) -> bool {
        false
    }

    fn is_variant_end(&self) -> bool {
        self.board().white().is_empty() || self.board().black().is_empty()
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        if self.board().black().is_empty() {
            Some(Outcome::Decisive { winner: White })
        } else if self.board().white().is_empty() {
            Some(Outcome::Decisive { winner: Black })
        } else {
            None
        }
    }
}

/// An Atomic Chess position.
#[derive(Clone, Debug)]
pub struct Atomic {
    board: Board,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for Atomic {
    fn default() -> Atomic {
        Atomic {
            board: Board::default(),
            turn: White,
            castling_rights: bitboard::CORNERS,
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for Atomic {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling_rights }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for Atomic {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> Atomic {
        do_move(&mut self.board, &mut self.turn, &mut self.castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);

        match *m {
            Move::Normal { capture: Some(_), to, .. }  | Move::EnPassant { to, .. } => {
                self.board.remove_piece_at(to);

                let explosion_radius = attacks::king_attacks(to) &
                                       self.board().occupied() &
                                       !self.board.pawns();

                for explosion in explosion_radius {
                    self.board.remove_piece_at(explosion);
                }

                self.castling_rights.discard_all(explosion_radius);
            },
            _ => ()
        }

        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Atomic, PositionError> {
        let pos = Atomic {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling_rights: setup.castling_rights(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        if pos.board().kings().count() > 2 {
            return Err(PositionError::TooManyKings);
        }

        if let Some(their_king) = pos.board().king_of(!pos.turn()) {
            if pos.king_attackers(their_king, pos.turn(), pos.board.occupied()).any() {
                return Err(PositionError::OppositeCheck);
            }
        } else {
            return Err(PositionError::NoKing { color: !pos.turn() });
        }

        validate_basic(&pos).map_or(Ok(pos), Err)
    }

    fn king_attackers(&self, square: Square, attacker: Color, occupied: Bitboard) -> Bitboard {
        if (attacks::king_attacks(square) & self.board().by_piece(&attacker.king())).any() {
            Bitboard(0)
        } else {
            self.board().attacks_to(square, attacker, occupied)
        }
    }

    fn castling_uncovers_rank_attack(&self, rook: Square, king_to: Square) -> bool {
        (attacks::king_attacks(king_to) & self.board().kings() & self.them()).is_empty() &&
        castling_uncovers_rank_attack(self, rook, king_to)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        // TODO: Atomic move generation could be much more efficient.
        gen_en_passant(self.board(), self.turn(), self.ep_square, None, moves);
        gen_non_king(self, !self.us(), moves);
        KingTag::gen_moves(self, !self.board().occupied(), moves);
        self.board().king_of(self.turn()).map(|king| gen_castling_moves(self, king, moves));

        util::swap_retain(moves, |m| {
            let after = self.clone().play_unchecked(m);
            if let Some(our_king) = after.board().king_of(self.turn()) {
                after.board().by_piece(&Role::King.of(!self.turn())).is_empty() ||
                after.king_attackers(our_king, !self.turn(), after.board.occupied()).is_empty()
            } else {
                false
            }
        });
    }

    fn is_insufficient_material(&self) -> bool {
        if self.is_variant_end() {
            return false;
        }

        if self.board().pawns().any() || self.board().queens().any() {
            return false;
        }

        if (self.board().knights() | self.board().bishops() | self.board().rooks()).count() == 1 {
            return true;
        }

        // Only knights.
        if self.board().occupied() == self.board().kings() | self.board().knights() {
            return self.board().knights().count() <= 2;
        }

        // Only bishops.
        if self.board().occupied() == self.board().kings() | self.board().bishops() {
            if (self.board().by_piece(&White.bishop()) & bitboard::DARK_SQUARES).is_empty() {
                return (self.board().by_piece(&Black.bishop()) & bitboard::LIGHT_SQUARES).is_empty()
            }
            if (self.board().by_piece(&White.bishop()) & bitboard::LIGHT_SQUARES).is_empty() {
                return (self.board().by_piece(&Black.bishop()) & bitboard::DARK_SQUARES).is_empty()
            }
        }

        false
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        for color in &[White, Black] {
            if self.board().by_piece(&color.king()).is_empty() {
                return Some(Outcome::Decisive { winner: !*color })
            }
        }
        None
    }

    fn is_variant_end(&self) -> bool {
        self.variant_outcome().is_some()
    }
}

/// A Racing kings position.
#[derive(Clone, Debug)]
pub struct RacingKings {
    board: Board,
    turn: Color,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for RacingKings {
    fn default() -> RacingKings {
        RacingKings {
            board: Board::racing_kings(),
            turn: White,
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for RacingKings {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { Bitboard(0) }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|s| is_relevant_ep(self, *s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for RacingKings {
    const TRACK_PROMOTED: bool = false;
    const KING_PROMOTIONS: bool = false;

    fn play_unchecked(mut self, m: &Move) -> RacingKings {
        let mut fake_castling_rights = Bitboard(0);
        do_move(&mut self.board, &mut self.turn, &mut fake_castling_rights,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);

        self
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<RacingKings, PositionError> {
        let pos = RacingKings {
            board: setup.board().clone(),
            turn: setup.turn(),
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        if setup.castling_rights().any() {
            return Err(PositionError::BadCastlingRights);
        }

        if pos.checkers().any() {
            return Err(PositionError::RacingKingsCheck);
        }

        if pos.turn().is_black() &&
           (pos.board().by_piece(&Black.king()) & Bitboard::rank(7)).any() &&
           (pos.board().by_piece(&White.king()) & Bitboard::rank(7)).any() {
            return Err(PositionError::RacingKingsOver);
        }

        validate_basic(&pos)
            .or_else(||validate_kings(&pos))
            .or_else(|| {
                if pos.board().pawns().any() {
                    return Some(PositionError::RacingKingsMaterial);
                }

                for color in &[White, Black] {
                    if pos.board().by_piece(&color.knight()).count() > 2 {
                        return Some(PositionError::RacingKingsMaterial);
                    }
                    if pos.board().by_piece(&color.bishop()).count() > 2 {
                        return Some(PositionError::RacingKingsMaterial);
                    }
                    if pos.board().by_piece(&color.rook()).count() > 2 {
                        return Some(PositionError::RacingKingsMaterial);
                    }
                    if pos.board().by_piece(&color.queen()).count() > 1 {
                        return Some(PositionError::RacingKingsMaterial);
                    }
                }

                None
            })
            .map_or(Ok(pos), Err)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        if !self.is_variant_end() {
            gen_standard(self, self.ep_square, moves);
        }

        // TODO: This could be more efficient.
        util::swap_retain(moves, |m| {
            let after = self.clone().play_unchecked(m);
            after.checkers().is_empty()
        });
    }

    fn is_insufficient_material(&self) -> bool {
        false
    }

    fn is_variant_end(&self) -> bool {
        let in_goal = self.board().kings() & Bitboard::rank(7);

        if in_goal.is_empty() {
            return false;
        }

        if self.turn().is_white() || (in_goal & self.board().black()).any() {
            return true;
        }

        // White has reached the backrank. Check if black can catch up.
        if let Some(black_king) = self.board().by_piece(&Black.king()).first() {
            for target in attacks::king_attacks(black_king) & Bitboard::rank(7) {
                if self.king_attackers(target, White, self.board.occupied()).is_empty() {
                    return false;
                }
            }
        }

        true
    }

    fn variant_outcome(&self) -> Option<Outcome> {
        if self.is_variant_end() {
            let in_goal = self.board().kings() & Bitboard::rank(7);
            if (in_goal & self.board().white()).any() && (in_goal & self.board().black()).any() {
                Some(Outcome::Draw)
            } else if (in_goal & self.board().white()).any() {
                Some(Outcome::Decisive { winner: White })
            } else {
                Some(Outcome::Decisive { winner: Black })
            }
        } else {
            None
        }
    }
}

fn do_move(board: &mut Board,
           turn: &mut Color,
           castling_rights: &mut Bitboard,
           ep_square: &mut Option<Square>,
           halfmove_clock: &mut u32,
           fullmoves: &mut u32,
           m: &Move) {
    let color = *turn;
    ep_square.take();
    *halfmove_clock = halfmove_clock.saturating_add(1);

    match *m {
        Move::Normal { role, from, capture, to, promotion } => {
            if role == Role::Pawn || capture.is_some() {
                *halfmove_clock = 0;
            }

            if role == Role::Pawn && square::distance(from, to) == 2 {
                *ep_square = from.offset(color.fold(8, -8));
            }

            if role == Role::King {
                castling_rights.discard_all(Bitboard::relative_rank(color, 0));
            } else {
                castling_rights.discard(from);
                castling_rights.discard(to);
            }

            let promoted = board.promoted().contains(from) || promotion.is_some();

            board.remove_piece_at(from);
            board.set_piece_at(to, promotion.map_or(role.of(color), |p| p.of(color)), promoted);
        },
        Move::Castle { king, rook } => {
            let rook_to = square::combine(
                if square::delta(rook, king) < 0 { square::D1 } else { square::F1 },
                rook);

            let king_to = square::combine(
                if square::delta(rook, king) < 0 { square::C1 } else { square::G1 },
                king);

            board.remove_piece_at(king);
            board.remove_piece_at(rook);
            board.set_piece_at(rook_to, color.rook(), false);
            board.set_piece_at(king_to, color.king(), false);

            castling_rights.discard_all(Bitboard::relative_rank(color, 0));
        },
        Move::EnPassant { from, to } => {
            board.remove_piece_at(square::combine(to, from)); // captured pawn
            board.remove_piece_at(from).map(|piece| board.set_piece_at(to, piece, false));
            *halfmove_clock = 0;
        },
        Move::Put { to, role } => {
            board.set_piece_at(to, Piece { color, role }, false);
        },
    }

    if color.is_black() {
        *fullmoves = fullmoves.saturating_add(1);
    }

    *turn = !color;
}

fn validate_basic<P: Position>(pos: &P) -> Option<PositionError> {
    if pos.board().occupied().is_empty() {
        return Some(PositionError::Empty)
    }

    if let Some(pockets) = pos.pockets() {
        if pos.board().pawns().count() + pockets.white.pawns as usize + pockets.black.pawns as usize > 16 {
            return Some(PositionError::TooManyPawns)
        }
        if pos.board().occupied().count() + pockets.count() as usize > 32 {
            return Some(PositionError::TooManyPieces)
        }
    } else {
        for color in &[White, Black] {
            if pos.board().by_color(*color).count() > 16 {
                return Some(PositionError::TooManyPieces)
            }
            if pos.board().by_piece(&color.pawn()).count() > 8 {
                return Some(PositionError::TooManyPawns)
            }
        }
    }

    if !(pos.board().pawns() & (Bitboard::rank(0) | Bitboard::rank(7))).is_empty() {
        return Some(PositionError::PawnsOnBackrank)
    }

    if setup::clean_castling_rights(pos, false) != pos.castling_rights() {
        return Some(PositionError::BadCastlingRights)
    }

    validate_ep(pos)
}

fn validate_ep<P: Position>(pos: &P) -> Option<PositionError> {
    if let Some(ep_square) = pos.ep_square() {
        if !Bitboard::relative_rank(pos.turn(), 5).contains(ep_square) {
            return Some(PositionError::InvalidEpSquare)
        }

        let fifth_rank_sq = ep_square.offset(pos.turn().fold(-8, 8)).expect("ep square is on sixth rank");
        let seventh_rank_sq  = ep_square.offset(pos.turn().fold(8, -8)).expect("ep square is on sixth rank");

        // The last move must have been a double pawn push. Check for the
        // presence of that pawn.
        if !pos.their(Role::Pawn).contains(fifth_rank_sq) {
            return Some(PositionError::InvalidEpSquare)
        }

        if pos.board().occupied().contains(ep_square) | pos.board().occupied().contains(seventh_rank_sq) {
            return Some(PositionError::InvalidEpSquare)
        }
    }

    None
}

fn validate_kings<P: Position>(pos: &P) -> Option<PositionError> {
    for color in &[White, Black] {
        if (pos.board().by_piece(&color.king()) & !pos.board().promoted()).is_empty() {
            return Some(PositionError::NoKing { color: *color })
        }
    }

    if pos.board().kings().count() > 2 {
        return Some(PositionError::TooManyKings)
    }

    if let Some(their_king) = pos.board().king_of(!pos.turn()) {
        if pos.king_attackers(their_king, pos.turn(), pos.board().occupied()).any() {
            return Some(PositionError::OppositeCheck)
        }
    }

    None
}

fn gen_standard<P: Position>(pos: &P, ep_square: Option<Square>, moves: &mut MoveList) {
    let our_king = pos.our(Role::King).first();
    gen_en_passant(pos.board(), pos.turn(), ep_square, our_king, moves);

    if let Some(king) = our_king {
        let checkers = pos.checkers();

        if checkers.is_empty() {
            let target = !pos.us();
            gen_safe_non_king(pos, target, king, moves);
            gen_safe_king(pos, target, moves);
            gen_castling_moves(pos, king, moves);
        } else {
            evasions(pos, king, checkers, moves);
        }
    } else {
        gen_non_king(pos, !pos.us(), moves);
    }
}

fn gen_non_king<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
    gen_pawn_moves(pos, target, moves, |_, _| true);
    KnightTag::gen_moves(pos, target, moves);
    BishopTag::gen_moves(pos, target, moves);
    RookTag::gen_moves(pos, target, moves);
    QueenTag::gen_moves(pos, target, moves);
}

fn gen_safe_non_king<P: Position>(pos: &P, target: Bitboard, king: Square, moves: &mut MoveList) {
    let blockers = slider_blockers(pos.board(), pos.them(), king);
    gen_pawn_moves(pos, target, moves, |from, to| {
        !blockers.contains(from) || attacks::aligned(from, to, king)
    });
    KnightTag::gen_safe_moves(pos, target, blockers, moves);
    BishopTag::gen_safe_moves(pos, target, king, blockers, moves);
    RookTag::gen_safe_moves(pos, target, king, blockers, moves);
    QueenTag::gen_safe_moves(pos, target, king, blockers, moves);
}

fn gen_safe_king<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
    for from in pos.our(Role::King) {
        moves.extend(
            (attacks::king_attacks(from) & target)
                .filter(|to| pos.board().attacks_to(*to, !pos.turn(), pos.board().occupied()).is_empty())
                .map(|to| Move::Normal {
                    role: Role::King,
                    from,
                    capture: pos.board().role_at(to),
                    to,
                    promotion: None,
                }));
    }
}

fn evasions<P: Position>(pos: &P, king: Square, checkers: Bitboard, moves: &mut MoveList) {
    let sliders = checkers & pos.board().sliders();

    let mut attacked = Bitboard(0);
    for checker in sliders {
        attacked |= attacks::ray(checker, king) ^ checker;
    }

    gen_safe_king(pos, !pos.us() & !attacked, moves);

    if let Some(checker) = checkers.single_square() {
        let target = attacks::between(king, checker).with(checker);
        gen_safe_non_king(pos, target, king, moves);
    }
}

fn gen_castling_moves<P: Position>(pos: &P, king: Square, moves: &mut MoveList) {
    'next_rook: for rook in pos.castling_rights() & Bitboard::relative_rank(pos.turn(), 0) {
        let (king_to, rook_to) = if king < rook {
            (pos.turn().fold(square::G1, square::G8),
             pos.turn().fold(square::F1, square::F8))
        } else {
            (pos.turn().fold(square::C1, square::C8),
             pos.turn().fold(square::D1, square::D8))
        };

        let king_path = attacks::between(king, king_to).with(king_to);
        let rook_path = attacks::between(rook, rook_to).with(rook_to);

        if ((pos.board().occupied() ^ king ^ rook) & (king_path | rook_path)).any() {
            continue;
        }

        for sq in king_path.with(king) {
            if pos.king_attackers(sq, !pos.turn(), pos.board().occupied() ^ king).any() {
                continue 'next_rook;
            }
        }

        if pos.castling_uncovers_rank_attack(rook, king_to) {
            continue;
        }

        moves.push(Move::Castle { king, rook });
    }
}

fn castling_uncovers_rank_attack<P: Position>(pos: &P, rook: Square, king_to: Square) -> bool {
    (attacks::rook_attacks(king_to, pos.board().occupied().without(rook)) &
     pos.them() & pos.board().rooks_and_queens() &
     Bitboard::rank(king_to.rank())).any()
}

trait Stepper {
    const ROLE: Role;

    fn attacks(from: Square) -> Bitboard;

    fn gen_moves<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
        for from in pos.our(Self::ROLE) {
            moves.extend((Self::attacks(from) & target).map(|to| {
                Move::Normal { role: Self::ROLE, from, capture: pos.board().role_at(to), to, promotion: None }
            }));
        }
    }

    fn gen_safe_moves<P: Position>(pos: &P, target: Bitboard, blockers: Bitboard, moves: &mut MoveList);
}

trait Slider {
    const ROLE: Role;

    fn attacks(from: Square, occupied: Bitboard) -> Bitboard;

    fn gen_moves<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
        for from in pos.our(Self::ROLE) {
            moves.extend((Self::attacks(from, pos.board().occupied()) & target).map(|to| {
                Move::Normal { role: Self::ROLE, from, capture: pos.board().role_at(to), to, promotion: None }
            }));
        }
    }

    fn gen_safe_moves<P: Position>(pos: &P, target: Bitboard, king: Square, blockers: Bitboard, moves: &mut MoveList) {
        let pieces = pos.our(Self::ROLE);

        for from in pieces & !blockers {
            moves.extend((Self::attacks(from, pos.board().occupied()) & target).map(|to| {
                Move::Normal { role: Self::ROLE, from, capture: pos.board().role_at(to), to, promotion: None, }
            }));
        }

        for from in pieces & blockers {
            let safe = Self::attacks(from, pos.board().occupied()) & target & attacks::ray(from, king);
            moves.extend(safe.map(|to| {
                Move::Normal { role: Self::ROLE, from, capture: pos.board().role_at(to), to, promotion: None, }
            }));
        }
    }
}

enum KingTag { }
enum KnightTag { }
enum BishopTag { }
enum RookTag { }
enum QueenTag { }

impl Stepper for KingTag {
    const ROLE: Role = Role::King;
    fn attacks(from: Square) -> Bitboard { attacks::king_attacks(from) }

    fn gen_safe_moves<P: Position>(pos: &P, target: Bitboard, _blockers: Bitboard, moves: &mut MoveList) {
        gen_safe_king(pos, target, moves);
    }
}

impl Stepper for KnightTag {
    const ROLE: Role = Role::Knight;
    fn attacks(from: Square) -> Bitboard { attacks::knight_attacks(from) }

    fn gen_safe_moves<P: Position>(pos: &P, target: Bitboard, blockers: Bitboard, moves: &mut MoveList) {
        for from in pos.our(Self::ROLE) & !blockers {
            moves.extend((Self::attacks(from) & target).map(|to| {
                Move::Normal { role: Self::ROLE, from, capture: pos.board().role_at(to), to, promotion: None, }
            }));
        }
    }
}

impl Slider for BishopTag {
    const ROLE: Role = Role::Bishop;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard { attacks::bishop_attacks(from, occupied) }
}

impl Slider for RookTag {
    const ROLE: Role = Role::Rook;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard { attacks::rook_attacks(from, occupied) }
}

impl Slider for QueenTag {
    const ROLE: Role = Role::Queen;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard { attacks::queen_attacks(from, occupied) }
}

fn gen_pawn_moves<P: Position, F>(pos: &P, target: Bitboard, moves: &mut MoveList, f: F)
    where F: Fn(Square, Square) -> bool
{
    let seventh = pos.our(Role::Pawn) & Bitboard::relative_rank(pos.turn(), 6);

    for from in pos.our(Role::Pawn) & !seventh {
        for to in attacks::pawn_attacks(pos.turn(), from) & pos.them() & target {
            if f(from, to) {
                moves.push(Move::Normal {
                    role: Role::Pawn,
                    from,
                    capture: pos.board().role_at(to),
                    to,
                    promotion: None
                });
            }
        }
    }

    for from in seventh {
        for to in attacks::pawn_attacks(pos.turn(), from) & pos.them() & target {
            if f(from, to) {
                push_promotions::<P>(moves, from, to, pos.board().role_at(to));
            }
        }
    }

    let single_moves = pos.our(Role::Pawn).relative_shift(pos.turn(), 8) &
                       !pos.board().occupied();

    let double_moves = single_moves.relative_shift(pos.turn(), 8) &
                       Bitboard::relative_rank(pos.turn(), 3) &
                       !pos.board().occupied();

    for to in single_moves & target & !bitboard::BACKRANKS {
        if let Some(from) = to.offset(pos.turn().fold(-8, 8)) {
            if f(from, to) {
                moves.push(Move::Normal { role: Role::Pawn, from, capture: None, to, promotion: None });
            }
        }
    }

    for to in single_moves & target & bitboard::BACKRANKS {
        if let Some(from) = to.offset(pos.turn().fold(-8, 8)) {
            if f(from, to) {
                push_promotions::<P>(moves, from, to, None);
            }
        }
    }

    for to in double_moves & target {
        if let Some(from) = to.offset(pos.turn().fold(-16, 16)) {
            if f(from, to) {
                moves.push(Move::Normal { role: Role::Pawn, from, capture: None, to, promotion: None });
            }
        }
    }
}

fn push_promotions<P: Position>(moves: &mut MoveList, from: Square, to: Square, capture: Option<Role>) {
    if P::KING_PROMOTIONS {
        moves.push(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::King) });
    }

    moves.push(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Queen) });
    moves.push(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Rook) });
    moves.push(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Bishop) });
    moves.push(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Knight) });
}

fn is_relevant_ep<P: Position>(pos: &P, ep_square: Square) -> bool {
    let mut moves = MoveList::new();
    gen_en_passant(pos.board(), pos.turn(), Some(ep_square), None, &mut moves);

    if moves.is_empty() {
        false
    } else {
        moves.clear();
        pos.legal_moves(&mut moves);
        moves.iter().any(|m| match *m {
            Move::EnPassant { to, .. } => to == ep_square,
            _ => false
        })
    }
}

fn gen_en_passant(board: &Board, turn: Color, ep_square: Option<Square>, safe_king: Option<Square>, moves: &mut MoveList) {
    if let Some(to) = ep_square {
        for from in board.pawns() & board.by_color(turn) & attacks::pawn_attacks(!turn, to) {
            // Ensure we are not in check after en passant.
            if let Some(king) = safe_king {
                let them = board.by_color(!turn);
                let captured = square::combine(to, from);

                // King in check, but not by the pawn.
                // Impossible from a series of legal moves.
                if (attacks::king_attacks(king) & them & board.kings()).any() ||
                   (attacks::knight_attacks(king) & them & board.knights()).any() ||
                   (attacks::pawn_attacks(turn, king) & them.without(captured) & board.pawns()).any() {
                    continue;
                }

                // Detect pins.
                let mut occupied = board.occupied();
                occupied.flip(from);
                occupied.flip(captured);
                occupied.add(to);

                if (attacks::rook_attacks(king, occupied) & them & board.rooks_and_queens()).any() ||
                   (attacks::bishop_attacks(king, occupied) & them & board.bishops_and_queens()).any() {
                    continue;
                }
            }

            moves.push(Move::EnPassant { from, to });
        }
    }
}

fn slider_blockers(board: &Board, enemy: Bitboard, king: Square) -> Bitboard {
    let snipers = (attacks::rook_attacks(king, Bitboard(0)) & board.rooks_and_queens()) |
                  (attacks::bishop_attacks(king, Bitboard(0)) & board.bishops_and_queens());

    let mut blockers = Bitboard(0);

    for sniper in snipers & enemy {
        let b = attacks::between(king, sniper) & board.occupied();

        if !b.more_than_one() {
            blockers.add_all(b);
        }
    }

    blockers
}

fn filter_san_candidates(role: Role, to: Square, moves: &mut MoveList) {
    util::swap_retain(moves, |m| match *m {
        Move::Normal { role: r, to: t, .. } | Move::Put { role: r, to: t } =>
            to == t && role == r,
        Move::Castle { rook, .. } => role == Role::King && to == rook,
        Move::EnPassant { to: t, .. } => role == Role::Pawn && t == to,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::Bencher;
    use fen::Fen;

    #[bench]
    fn bench_generate_moves(b: &mut Bencher) {
        let fen = "rn1qkb1r/pbp2ppp/1p2p3/3n4/8/2N2NP1/PP1PPPBP/R1BQ1RK1 b kq -";
        let pos: Chess = fen.parse::<Fen>().expect("valid fen")
                            .position().expect("legal position");

        b.iter(|| {
            let mut moves = MoveList::new();
            pos.legal_moves(&mut moves);
            assert_eq!(moves.len(), 39);
        })
    }

    #[bench]
    fn bench_play_unchecked(b: &mut Bencher) {
        let fen = "rn1qkb1r/pbp2ppp/1p2p3/3n4/8/2N2NP1/PP1PPPBP/R1BQ1RK1 b kq -";
        let pos: Chess = fen.parse::<Fen>().expect("valid fen")
                            .position().expect("legal position");
        let m = Move::Normal {
            role: Role::Bishop,
            from: square::F8,
            capture: None,
            to: square::E7,
            promotion: None,
        };

        b.iter(|| {
            let after = pos.clone().play_unchecked(&m);
            assert_eq!(after.turn(), White);
        });
    }
}
