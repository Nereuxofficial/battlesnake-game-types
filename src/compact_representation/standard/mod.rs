use crate::compact_representation::core::CellNum as CN;
use crate::impl_common_board_traits;
use crate::types::{
    build_snake_id_map, FoodGettableGame, HazardQueryableGame, HazardSettableGame,
    HeadGettableGame, HealthGettableGame, LengthGettableGame, PositionGettableGame,
    RandomReasonableMovesGame, SizeDeterminableGame, SnakeIDGettableGame, SnakeIDMap, SnakeId,
    VictorDeterminableGame, YouDeterminableGame, Action,
};
/// you almost certainly want to use the `convert_from_game` method to
/// cast from a json represention to a `CellBoard`
use crate::types::{NeighborDeterminableGame, SnakeBodyGettableGame};
use crate::wire_representation::Game;
use itertools::Itertools;
use rand::prelude::IteratorRandom;
use rand::Rng;
use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Display;

use crate::{
    types::{Move, SimulableGame, SimulatorInstruments},
    wire_representation::Position,
};

use super::core::{Cell, simulate_with_moves, EvaluateMode};
use super::core::CellIndex;
use super::core::CellBoard as CCB;
use super::core::{DOUBLE_STACK, TRIPLE_STACK};

/// A compact board representation that is significantly faster for simulation than
/// `battlesnake_game_types::wire_representation::Game`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CellBoard<T: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize> {
    embedded: CCB<T, BOARD_SIZE, MAX_SNAKES>,
}

impl_common_board_traits!(CellBoard);

/// 7x7 board with 4 snakes
pub type CellBoard4Snakes7x7 = CellBoard<u8, { 7 * 7 }, 4>;

/// Used to represent the standard 11x11 game with up to 4 snakes.
pub type CellBoard4Snakes11x11 = CellBoard<u8, { 11 * 11 }, 4>;

/// Used to represent the a 15x15 board with up to 4 snakes. This is the biggest board size that
/// can still use u8s
pub type CellBoard8Snakes15x15 = CellBoard<u8, { 15 * 15 }, 8>;

/// Used to represent the largest UI Selectable board with 8 snakes.
pub type CellBoard8Snakes25x25 = CellBoard<u16, { 25 * 25 }, 8>;

/// Used to represent an absolutely silly game board
pub type CellBoard16Snakes50x50 = CellBoard<u16, { 50 * 50 }, 16>;

/// Enum that holds a Cell Board sized right for the given game
#[derive(Debug)]
pub enum BestCellBoard {
    #[allow(missing_docs)]
    Tiny(Box<CellBoard4Snakes7x7>),
    #[allow(missing_docs)]
    Standard(Box<CellBoard4Snakes11x11>),
    #[allow(missing_docs)]
    LargestU8(Box<CellBoard8Snakes15x15>),
    #[allow(missing_docs)]
    Large(Box<CellBoard8Snakes25x25>),
    #[allow(missing_docs)]
    Silly(Box<CellBoard16Snakes50x50>),
}

fn get_snake_id(
    snake: &crate::wire_representation::BattleSnake,
    snake_ids: &SnakeIDMap,
) -> Option<SnakeId> {
    if snake.health == 0 {
        None
    } else {
        Some(*snake_ids.get(&snake.id).unwrap())
    }
}

impl<T: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    CellBoard<T, BOARD_SIZE, MAX_SNAKES>
{
    /// Builds a cellboard from a given game, will return an error if the game doesn't match
    /// the provided BOARD_SIZE or MAX_SNAKES. You are encouraged to use `CellBoard4Snakes11x11`
    /// for the common game layout
    pub fn convert_from_game(game: Game, snake_ids: &SnakeIDMap) -> Result<Self, Box<dyn Error>> {
        if game.game.ruleset.name == "wrapped" {
            return Err("Wrapped games are not supported".into());
        }

        if game.board.width * game.board.height > BOARD_SIZE as u32 {
            return Err("game size doesn't fit in the given board size".into());
        }

        if game.board.snakes.len() > MAX_SNAKES {
            return Err("too many snakes".into());
        }

        for snake in &game.board.snakes {
            let counts = &snake.body.iter().counts();
            if counts.values().any(|v| *v == TRIPLE_STACK) && counts.len() != 1 {
                return Err(format!("snake {} has a bad body stack (3 segs on same square and more than one unique position)", snake.id).into());
            }
        }
        let width = game.board.width as u8;

        let mut cells = [Cell::empty(); BOARD_SIZE];
        let mut healths: [u8; MAX_SNAKES] = [0; MAX_SNAKES];
        let mut heads: [CellIndex<T>; MAX_SNAKES] = [CellIndex::from_i32(0); MAX_SNAKES];
        let mut lengths: [u16; MAX_SNAKES] = [0; MAX_SNAKES];

        for snake in &game.board.snakes {
            let snake_id = match get_snake_id(snake, snake_ids) {
                Some(value) => value,
                None => continue,
            };

            healths[snake_id.0 as usize] = snake.health as u8;
            if snake.health == 0 {
                continue;
            }
            lengths[snake_id.0 as usize] = snake.body.len() as u16;

            let counts = &snake.body.iter().counts();

            let head_idx = CellIndex::new(snake.head, width);
            let mut next_index = head_idx;
            for (idx, pos) in snake.body.iter().unique().enumerate() {
                let cell_idx = CellIndex::new(*pos, width);
                let count = counts.get(pos).unwrap();
                if idx == 0 {
                    assert!(cell_idx == head_idx);
                    heads[snake_id.0 as usize] = head_idx;
                }
                cells[cell_idx.0.as_usize()] = if *count == TRIPLE_STACK {
                    Cell::make_triple_stacked_piece(snake_id)
                } else if *pos == snake.head {
                    // head can never be doubled, so let's assert it here, the cost of
                    // one comparison is worth the saftey imo
                    assert!(*count != DOUBLE_STACK);
                    let tail_index = CellIndex::new(*snake.body.back().unwrap(), width);
                    Cell::make_snake_head(snake_id, tail_index)
                } else if *count == DOUBLE_STACK {
                    Cell::make_double_stacked_piece(snake_id, next_index)
                } else {
                    Cell::make_body_piece(snake_id, next_index)
                };
                next_index = cell_idx;
            }
        }
        for y in 0..game.board.height {
            for x in 0..game.board.width {
                let position = Position {
                    x: x as i32,
                    y: y as i32,
                };
                let cell_idx: CellIndex<T> = CellIndex::new(position, width);
                if game.board.hazards.contains(&position) {
                    cells[cell_idx.0.as_usize()].set_hazard();
                }
                if game.board.food.contains(&position) {
                    cells[cell_idx.0.as_usize()].set_food();
                }
            }
        }

        let embedded = CCB::new(
           game
               .game
               .ruleset
               .settings
               .as_ref()
               .map(|s| s.hazard_damage_per_turn)
               .unwrap_or(15) as u8,
           cells,
           healths,
           heads,
           lengths,
         game.board.width as u8,
        );

        Ok(CellBoard {
            embedded,
        })
    }

    fn off_board(&self, new_head: Position) -> bool {
        new_head.x < 0
            || new_head.x >= self.embedded.get_actual_width() as i32
            || new_head.y < 0
            || new_head.y >= self.embedded.get_actual_height() as i32
    }
}

impl<T: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize> RandomReasonableMovesGame
    for CellBoard<T, BOARD_SIZE, MAX_SNAKES>
{
    fn random_reasonable_move_for_each_snake<'a>(
        &'a self, rng: &'a mut impl Rng,
    ) -> Box<dyn std::iter::Iterator<Item = (SnakeId, Move)> + 'a> {
        let width = self.embedded.get_actual_width();
        Box::new(
            self.embedded.iter_healths()
                .enumerate()
                .filter(|(_, health)| **health > 0)
                .map(move |(idx, _)| {
                    let head_pos = self.get_head_as_position(&SnakeId(idx as u8));

                    let mv = IntoIterator::into_iter(Move::all())
                        .filter(|mv| {
                            let new_head = head_pos.add_vec(mv.to_vector());
                            let ci = CellIndex::new(head_pos.add_vec(mv.to_vector()), width);

                            !self.off_board(new_head) &&
                            !self.embedded.cell_is_body(ci) && !self.embedded.cell_is_snake_head(ci)
                        })
                        .choose(rng)
                        .unwrap_or(Move::Up);
                    (SnakeId(idx as u8), mv)
                }),
        )
    }
}

impl<T: SimulatorInstruments, N: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    SimulableGame<T, MAX_SNAKES> for CellBoard<N, BOARD_SIZE, MAX_SNAKES>
{
    #[allow(clippy::type_complexity)]
    fn simulate_with_moves<S>(
        &self,
        instruments: &T,
        snake_ids_and_moves: impl IntoIterator<Item = (Self::SnakeIDType, S)>,
    ) -> Box<dyn Iterator<Item = (Action<MAX_SNAKES>, Self)> + '_>
    where
        S: Borrow<[Move]>,
    {
        Box::new(simulate_with_moves(&self.embedded, instruments, snake_ids_and_moves, EvaluateMode::Standard).map(|v| {
            let (action, board) = v;
            (action, Self { embedded: board})
        }))
    }
}

impl<T: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize> NeighborDeterminableGame
    for CellBoard<T, BOARD_SIZE, MAX_SNAKES>
{
    fn possible_moves(
        &self,
        pos: &Self::NativePositionType,
    ) -> Vec<(Move, Self::NativePositionType)> {
        let width = self.embedded.get_actual_width();

        Move::all()
            .iter()
            .map(|mv| {
                let head_pos = pos.into_position(width);
                let new_head = head_pos.add_vec(mv.to_vector());
                let ci = CellIndex::new(new_head, width);

                (*mv, new_head, ci)
            })
            .filter(|(_mv, new_head, _)| !self.off_board(*new_head))
            .map(|(mv, _, ci)| (mv, ci))
            .collect()
    }

    fn neighbors(&self, pos: &Self::NativePositionType) -> std::vec::Vec<Self::NativePositionType> {
        let width = self.embedded.get_actual_width();

        Move::all()
            .iter()
            .map(|mv| {
                let head_pos = pos.into_position(width);
                let new_head = head_pos.add_vec(mv.to_vector());
                let ci = CellIndex::new(new_head, width);

                (new_head, ci)
            })
            .filter(|(new_head, _)| !self.off_board(*new_head))
            .map(|(_, ci)| ci)
            .collect()
    }
}

/// Trait to get the best sized cellboard for the given game. It returns the smallest Compact board
/// that has enough room to fit the given Wire game. If the game can't fit in any of our Compact
/// boards we panic. However the largest board available is MUCH larger than the biggest selectable
/// board in the Battlesnake UI
pub trait ToBestCellBoard {
    #[allow(missing_docs)]
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>>;
}

impl ToBestCellBoard for Game {
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>> {
        let dimension = self.board.width;
        let num_snakes = self.board.snakes.len();
        let id_map = build_snake_id_map(&self);

        let best_board = if dimension <= 7 && num_snakes <= 4 {
            BestCellBoard::Tiny(Box::new(CellBoard4Snakes7x7::convert_from_game(
                self, &id_map,
            )?))
        } else if dimension <= 11 && num_snakes <= 4 {
            BestCellBoard::Standard(Box::new(CellBoard4Snakes11x11::convert_from_game(
                self, &id_map,
            )?))
        } else if dimension <= 15 && num_snakes <= 8 {
            BestCellBoard::LargestU8(Box::new(CellBoard8Snakes15x15::convert_from_game(
                self, &id_map,
            )?))
        } else if dimension <= 25 && num_snakes <= 8 {
            BestCellBoard::Large(Box::new(CellBoard8Snakes25x25::convert_from_game(
                self, &id_map,
            )?))
        } else if dimension <= 50 && num_snakes <= 16 {
            BestCellBoard::Silly(Box::new(CellBoard16Snakes50x50::convert_from_game(
                self, &id_map,
            )?))
        } else {
            panic!("No board was big enough")
        };

        Ok(best_board)
    }
}
#[cfg(test)]
mod test {

    use super::*;
    use crate::{
        game_fixture,
        types::{build_snake_id_map},
        wire_representation::Game as DEGame,
    };
    #[derive(Debug)]
    struct Instruments;
    impl SimulatorInstruments for Instruments {
        fn observe_simulation(&self, _: std::time::Duration) {}
    }

    #[test]
    fn test_compact_board_conversion() {
        let start_of_game_fixture =
            game_fixture(include_str!("../../../fixtures/start_of_game.json"));
        let converted = Game::to_best_cell_board(start_of_game_fixture);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::Standard(_) => {}
            _ => panic!("expected standard board"),
        }

        let tiny_board = game_fixture(include_str!("../../../fixtures/7x7board.json"));
        let converted = Game::to_best_cell_board(tiny_board);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::Tiny(_) => {}
            _ => panic!("expected standard board"),
        }
    }


    #[test]
    fn test_head_gettable() {
        let game_fixture = include_str!("../../../fixtures/late_stage.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        assert_eq!(
            compact.get_head_as_position(&SnakeId(0)),
            Position { x: 4, y: 6 }
        );
        assert_eq!(
            compact.get_head_as_native_position(&SnakeId(0)),
            CellIndex(6 * 11 + 4)
        );
    }

    #[test]
    fn test_tail_collision() {
        let game_fixture = include_str!("../../../fixtures/start_of_game.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let mut compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        let moves = [
            Move::Left,
            Move::Down,
            Move::Right,
            Move::Up,
            Move::Left,
            Move::Down,
        ];
        let instruments = Instruments;
        eprintln!("{}", compact);
        for mv in moves {
            let res = compact
                .simulate_with_moves(&instruments, vec![(SnakeId(0), [mv].as_slice())])
                .collect_vec();
            compact = res[0].1;
            eprintln!("{}", compact);
        }
        assert!(compact.get_health(&SnakeId(0)) > 0);
    }

    #[test]
    fn test_set_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
    }

    #[test]
    fn test_clear_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_food());
        assert!(!c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
        let mut c: Cell<u8> = Cell::make_double_stacked_piece(SnakeId(0), CellIndex(0));
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_body());
        assert!(!c.is_hazard());
    }

    #[test]
    fn test_remove() {
        let mut c: Cell<u8> = Cell::make_body_piece(SnakeId(3), CellIndex(17));
        c.remove();
        c.set_hazard();
        assert!(c.is_empty());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }
    #[test]
    fn test_set_food() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }

    #[test]
    fn test_set_head() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_head(SnakeId(3), CellIndex(17));
        c.set_hazard();
        assert!(c.is_head());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().unwrap() == SnakeId(3));
        assert!(c.get_idx() == CellIndex(17));
    }
}
