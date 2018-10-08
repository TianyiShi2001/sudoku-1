#![allow(unused)]
#![warn(unused_variables, unused_mut, unused_must_use)]
#![allow(missing_docs)]
use ::Sudoku;
use bitset::{Set, Iter as SetIter};
use board::Candidate;
use helper::{HouseArray, CellArray, DigitArray, Unsolvable};
use consts::*;
use board::*;
use strategy::{
	deduction::{Deduction, Deductions},
	strategies::Strategy,
};

type EliminationsRange = ::std::ops::Range<usize>;
type _Deduction = Deduction<EliminationsRange>;

/// The `StrategySolver` is the struct for solving sudokus with
/// strategies that humans commonly apply.
///
/// It is built from a single `Sudoku` for which it stores the current
/// state and the history of applied strategies. It can find hints
/// or solve the `Sudoku` completely and return the solution path.
/// From the solving path, the difficulty can be graded.

// To allow for the above functionality, this struct contains caches
// of various properties of the sudoku grid. The caches are lazily updated
// on demand. This avoids both unnecessary and repetitive work.
//
// Two histories are kept:
// 1. A list of all strategies that were successfully used to deduce or eliminate entries
// 2. Two lists for all entered and all eliminated digit-cell-entries
//    The former also includes initial clues.
//
// The 1st list is for reporting the sequence of strategies applied
// The 2nd list is for the updating of internal caches.
// It is kept simple to offer an easy interface and can contain duplicates.
//
// These two histories can contain overlapping information and the
// 1st one can also contain references to the 2nd but not vice versa.
#[derive(Debug, Clone)]
pub struct StrategySolver {
	pub(crate) deductions: Vec<_Deduction>,
	pub(crate) deduced_entries: Vec<Candidate>,
	pub(crate) eliminated_entries: Vec<Candidate>,
	pub(crate) n_solved: u8, // deduced_entries can contain duplicates so a separate counter is necessary
	// current state of the sudoku
	// for when it's faster to recompute from the end state
	// than update through the new entries
	pub(crate) grid: State<Sudoku>,
	// TODO: combine states that are updated together
	// Mask of possible numbers in cell
	pub(crate) cell_poss_digits: State<CellArray<Set<Digit>>>,
	// Mask of solved digits in house
	pub(crate) house_solved_digits: State<HouseArray<Set<Digit>>>,
	// Mask of possible positions for a house and number
	pub(crate) house_poss_positions: State<HouseArray<DigitArray<Set<Position<House>>>>>,
}

impl StrategySolver {
	pub fn from_sudoku(sudoku: Sudoku) -> StrategySolver {
		let deduced_entries = sudoku.iter()
			.enumerate()
			.filter_map(|(cell, opt_num)| {
				opt_num.map(|digit| Candidate::new(cell as u8, digit))
			}).collect();
		StrategySolver {
			deductions: vec![],
			deduced_entries,
			eliminated_entries: vec![],
			n_solved: 0,
			grid: State::from(Sudoku([0; 81])),
			cell_poss_digits: State::from(CellArray([Set::ALL; 81])),
			house_solved_digits: State::from(HouseArray([Set::NONE; 27])),
			house_poss_positions: State::from(HouseArray([DigitArray([Set::ALL; 9]); 27])),
		}

	}

	/// Returns the current state of the Sudoku
	pub fn to_sudoku(&mut self) -> Sudoku {
		self.update_grid();
		self.grid.state
	}

	/// Returns the current state of the Sudoku
	pub fn grid_state(&mut self) -> [CellState; 81] {
		let mut grid = [CellState::Candidates(Set::NONE); 81];
		self.update_grid();
		// TODO: continue despite error
		let _ = self._update_cell_poss_house_solved(false);

		for (cell, &digits) in self.cell_poss_digits.state.iter().enumerate() {
			grid[cell] = CellState::Candidates(digits);
		}
		for (cell, &digit) in self.grid.state.0.iter().enumerate().filter(|(_, &digit)| digit != 0) {
			grid[cell] = CellState::Digit(Digit::new(digit));
		}
		grid
	}

	/// Returns the current state of the cell
	pub fn cell_state(&mut self, cell: Cell) -> CellState {
		self.update_grid();
		let _ = self._update_cell_poss_house_solved(false);

		let digit = self.grid.state.0[cell.as_index()];
		if digit != 0 {
			CellState::Digit(Digit::new(digit))
		} else {
			let digits = self.cell_poss_digits.state[cell];
			CellState::Candidates(digits)
		}
	}

	/// Try to insert the given candidate. Fails, if the cell already contains a digit.
	pub fn insert_candidate(&mut self, candidate: Candidate) -> Result<(), ()> {
		self.update_grid();
		Self::push_new_candidate(
			&mut self.grid.state,
			&mut self.deduced_entries,
			candidate,
			&mut self.deductions,
			Deduction::Given(candidate)
		)
		.map_err(|Unsolvable| ())?;

		Ok(())
	}

	fn into_deductions(self) -> Deductions {
		let Self { deductions, deduced_entries, eliminated_entries, .. } = self;
		Deductions { deductions, deduced_entries, eliminated_entries }
	}

	fn update_grid(&mut self) {
		for &Candidate { cell, digit } in &self.deduced_entries {
			self.grid.state.0[cell.as_index()] = digit.get();
		}
	}

	/// Try to solve the sudoku using the given `strategies`. Returns a `Result` of the sudoku and a struct containing the series of deductions.
	/// If a solution was found, `Ok(..)` is returned, otherwise `Err(..)`.
	pub fn solve(mut self, strategies: &[Strategy]) -> Result<(Sudoku, Deductions), (Sudoku, Deductions)> {
		self.try_solve(strategies);
		self.update_grid();
		match self.is_solved() {
			true => Ok((self.grid.state, self.into_deductions())),
			false => Err((self.grid.state, self.into_deductions())),
		}
	}

	// FIXME: change name
	/// Try to solve the sudoku using the given `strategies`. Returns `true` if new deductions were made.
	pub fn try_solve(&mut self, strategies: &[Strategy]) -> bool {
		// first strategy can be optimized
		let (first, rest) = match strategies.split_first() {
			Some(tup) => tup,
			// no chance without strategies
			None => return false,
		};
		let lens = (self.deduced_entries.len(), self.eliminated_entries.len());
		'outer: loop {
			if self.is_solved() {
				break
			}

			let n_deductions = self.deduced_entries.len();
			let n_eliminated = self.eliminated_entries.len();
			if first.deduce_all(self, true).is_err() { break };
			if self.deduced_entries.len() > n_deductions {
				continue 'outer
			}

			for strategy in rest {
				if strategy.deduce_one(self).is_err() {
					break;
				};
				if self.deduced_entries.len() > n_deductions || self.eliminated_entries.len() > n_eliminated {
					continue 'outer
				}
			}
			break
		}
		lens < (self.deduced_entries.len(), self.eliminated_entries.len())
	}

	pub fn is_solved(&self) -> bool {
		self.n_solved == 81
	}

	fn update_cell_poss_house_solved(&mut self) -> Result<(), Unsolvable> {
		self._update_cell_poss_house_solved(false)
	}

	pub(crate) fn _update_cell_poss_house_solved(&mut self, find_naked_singles: bool) -> Result<(), Unsolvable> {
		{
			let (_, le_cp, cell_poss) = self.cell_poss_digits.get_mut();

			for &candidate in &self.eliminated_entries[*le_cp as _..] {
				let impossibles = candidate.digit_set();

				// deductions made here may conflict with entries already in the queue
				// in the queue. In that case the sudoku is impossible.
				Self::remove_impossibilities(&mut self.grid.state, cell_poss, candidate.cell, impossibles, &mut self.deduced_entries, &mut self.deductions, find_naked_singles)?;
			}
			*le_cp = self.eliminated_entries.len() as _;
		}

		self.insert_entries(find_naked_singles)
	}

	fn update_house_poss_positions(&mut self) -> Result<(), Unsolvable> {
		// TODO: this has to do massive amounts of work
		//       may just be easier to recompute from full grid every time

		let (ld, le, house_poss_positions) = self.house_poss_positions.get_mut();
		// remove now impossible positions from list
		for candidate in &self.eliminated_entries[*le as usize ..] {
			let cell = candidate.cell;
			let row_pos = cell.row_pos();
			let col_pos = cell.col_pos();
			let block_pos = cell.block_pos();
			// just 1 digit
			let digit = candidate.digit;

			house_poss_positions[cell.row()][digit].remove(row_pos.as_set());
			house_poss_positions[cell.col()][digit].remove(col_pos.as_set());
			house_poss_positions[cell.block()][digit].remove(block_pos.as_set());
		}
		*le = self.eliminated_entries.len() as _;

		for candidate in &self.deduced_entries[*ld as usize ..] {
			let cell = candidate.cell;
			let digit = candidate.digit;

			// remove digit from every house pos in all neighboring cells
			for cell in cell.neighbors() {
				let row_pos = cell.row_pos();
				let col_pos = cell.col_pos();
				let block_pos = cell.block_pos();
				house_poss_positions[cell.row()][digit].remove(row_pos.as_set());
				house_poss_positions[cell.col()][digit].remove(col_pos.as_set());
				house_poss_positions[cell.block()][digit].remove(block_pos.as_set());
			}

			let row = cell.row();
			let col = cell.col();
			let block = cell.block();
			let row_pos = cell.row_pos();
			let col_pos = cell.col_pos();
			let block_pos = cell.block_pos();

			// remove candidate pos as possible place for all nums
			for digit in Digit::all() {
				house_poss_positions[row][digit].remove(row_pos.as_set());
				house_poss_positions[col][digit].remove(col_pos.as_set());
				house_poss_positions[block][digit].remove(block_pos.as_set());
			}

			// remove all pos as possible place for candidate digit
			house_poss_positions[row][digit] = Set::NONE;
			house_poss_positions[col][digit] = Set::NONE;
			house_poss_positions[block][digit] = Set::NONE;
		}
		*ld = self.deduced_entries.len() as _;
		Ok(())
	}

	#[inline(always)]
	fn insert_entries(&mut self, find_naked_singles: bool) -> Result<(), Unsolvable> {
		// code hereafter depends on this
		// but it's not necessary in general
		assert!(self.cell_poss_digits.next_deduced == self.house_solved_digits.next_deduced);

		// TODO: Delete?
		// start off with batch insertion so every cell is visited at least once
		// because other strategies may have touched their possibilities which singly_insertion may miss
		self.batch_insert_entries(find_naked_singles)?;
		loop {
			match self.deduced_entries.len() - self.cell_poss_digits.next_deduced as usize {
				0 => break Ok(()),
				1...4 => self.insert_entries_singly(find_naked_singles)?,
				_ => self.batch_insert_entries(find_naked_singles)?,
			}
		}
	}

	// for each candidate in the stack, insert it (if cell is unsolved)
	// and then remove possibility from each cell neighboring it in all
	// houses (rows, cols, fields) eagerly
	// check for naked singles and impossible cells during this check
	fn insert_entries_singly(&mut self, find_naked_singles: bool) -> Result<(), Unsolvable> {
		let (ld_cp, _, cell_poss_digits) = self.cell_poss_digits.get_mut();
		let (ld_zs, _, house_solved_digits) = self.house_solved_digits.get_mut();

		loop {
			if self.deduced_entries.len() <= *ld_cp as usize { break }
			let candidate = self.deduced_entries[*ld_cp as usize];
			*ld_cp += 1;
			*ld_zs += 1;
			let candidate_mask = candidate.digit_set();
			// cell already solved from previous candidate in stack, skip
			if cell_poss_digits[candidate.cell].is_empty() { continue }

			// is candidate still possible?
			if (cell_poss_digits[candidate.cell] & candidate_mask).is_empty() {
				return Err(Unsolvable);
			}

			Self::_insert_candidate_cp_zs(candidate, &mut self.n_solved, cell_poss_digits, house_solved_digits);
			for cell in candidate.cell.neighbors() {
				if candidate_mask.overlaps(cell_poss_digits[cell]) {
					Self::remove_impossibilities(&mut self.grid.state, cell_poss_digits, cell, candidate_mask, &mut self.deduced_entries, &mut self.deductions, find_naked_singles)?;
				};
			}

			// found a lot of naked singles, switch to batch insertion
			if self.deduced_entries.len() - *ld_cp as usize > 4 { return Ok(()) }
		}
		Ok(())
	}

	#[inline]
	fn _insert_candidate_cp_zs(
		candidate: Candidate,
		n_solved: &mut u8,
		cell_poss_digits: &mut CellArray<Set<Digit>>,
		house_solved_digits: &mut HouseArray<Set<Digit>>
	) {
		*n_solved += 1;
		cell_poss_digits[candidate.cell] = Set::NONE;
		house_solved_digits[candidate.row()] |= candidate.digit_set();
		house_solved_digits[candidate.col()] |= candidate.digit_set();
		house_solved_digits[candidate.block()] |= candidate.digit_set();
	}

	fn batch_insert_entries(&mut self, find_naked_singles: bool) -> Result<(), Unsolvable> {
		let (ld_cp, _, cell_poss_digits) = self.cell_poss_digits.get_mut();
		let (ld_zs, _, house_solved_digits) = self.house_solved_digits.get_mut();
		while self.deduced_entries.len() > *ld_cp as usize {
			let candidate = self.deduced_entries[*ld_cp as usize];
			*ld_cp += 1;
			*ld_zs += 1;
			// cell already solved from previous candidate in stack, skip
			if cell_poss_digits[candidate.cell].is_empty() { continue }

			let candidate_mask = candidate.digit_set();

			// is candidate still possible?
			// have to check house possibilities, because cell possibility
			// is temporarily out of date
			if house_solved_digits[candidate.row()].overlaps(candidate_mask)
			|| house_solved_digits[candidate.col()].overlaps(candidate_mask)
			|| house_solved_digits[candidate.block()].overlaps(candidate_mask)
			{
				return Err(Unsolvable);
			}

			Self::_insert_candidate_cp_zs(candidate, &mut self.n_solved, cell_poss_digits, house_solved_digits);
		}

		// update cell possibilities from house masks
		for cell in Cell::all() {
			if cell_poss_digits[cell].is_empty() { continue }
			let houses_mask = house_solved_digits[cell.row()]
				| house_solved_digits[cell.col()]
				| house_solved_digits[cell.block()];

			Self::remove_impossibilities(&mut self.grid.state, cell_poss_digits, cell, houses_mask, &mut self.deduced_entries, &mut self.deductions, find_naked_singles)?;
		}
		Ok(())
	}

	// remove impossible digits from masks for given cell
	// also check for naked singles and impossibility of sudoku
	fn remove_impossibilities(
		sudoku: &mut Sudoku,
		cell_poss_digits: &mut CellArray<Set<Digit>>,
		cell: Cell,
		impossible: Set<Digit>,
		deduced_entries: &mut Vec<Candidate>,
		deductions: &mut Vec<_Deduction>,
		find_naked_singles: bool,
	) -> Result<(), Unsolvable> {
		let cell_mask = &mut cell_poss_digits[cell];
		cell_mask.remove(impossible);

		if find_naked_singles {
			if let Some(digit) = cell_mask.unique()? {
				let candidate = Candidate { cell, digit };
				Self::push_new_candidate(sudoku, deduced_entries, candidate, deductions, Deduction::NakedSingles(candidate))?;
			}
		} else {
			if cell_mask.is_empty() {
				return Err(Unsolvable)
			}
		}
		Ok(())
	}

	fn push_new_candidate(
		sudoku: &mut Sudoku,
		deduced_entries: &mut Vec<Candidate>,
		candidate: Candidate,
		deductions: &mut Vec<_Deduction>,
		strategy: _Deduction // either a user-given or naked or hidden singles
	) -> Result<(), Unsolvable> {

		#[cfg(debug_assertions)]
		{
			use self::Deduction::*;
			match strategy {
				NakedSingles(..) | HiddenSingles(..) | Given(_) => (),
				_ => panic!("Internal error: Called push_new_candidate with wrong strategy type")
			};
		}

		let old_num = &mut sudoku.0[candidate.cell.as_index()];
		match *old_num {
			n if n == candidate.digit.get() => return Ok(()),  // previously solved
			0 => (),                              // not solved
			_ => return Err(Unsolvable),          // conflict
		}
		*old_num = candidate.digit.get();
		deduced_entries.push(candidate);
		deductions.push(strategy);
		Ok(())
	}

	///////////////////////////////////////////////////////////////////////////////////////////////////////////////////
	////////      Strategies
	///////////////////////////////////////////////////////////////////////////////////////////////////////////////////

	pub(crate) fn find_naked_singles(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
		self.update_cell_poss_house_solved()?;

		for (cell, poss_digits) in Cell::all().zip(self.cell_poss_digits.state.iter()) {
			// if Err(_), then it's Set::NONE and the cell is already solved (or impossible)
			// skip in that case (via unwrap_or(None))
			if let Some(digit) = poss_digits.unique().unwrap_or(None) {
				let candidate = Candidate { cell, digit };

				Self::push_new_candidate(&mut self.grid.state, &mut self.deduced_entries, candidate, &mut self.deductions, Deduction::NakedSingles(candidate))?;
				self.deduced_entries.push(candidate);
				if stop_after_first {
					break
				}
			}
		}
		// call update again so newly found entries are inserted
		self.update_cell_poss_house_solved()
	}

	pub(crate) fn find_hidden_singles(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
		// TODO: remove auto-deducing naked singles inside update procedure
		self.update_cell_poss_house_solved()?;

		for house in House::all() {
			let mut unsolved: Set<Digit> = Set::NONE;
			let mut multiple_unsolved = Set::NONE;

			let cells = house.cells();
			for cell in cells {
				let poss_digits = self.cell_poss_digits.state[cell];
				multiple_unsolved |= unsolved & poss_digits;
				unsolved |= poss_digits;
			}
			if unsolved | self.house_solved_digits.state[house] != Set::ALL {
				return Err(Unsolvable);
			}

			let mut singles = unsolved.without(multiple_unsolved);
			if singles.is_empty() { continue }

			for cell in cells {
				let mask = self.cell_poss_digits.state[cell];

				if let Ok(maybe_unique) = (mask & singles).unique() {
					let digit = maybe_unique.ok_or(Unsolvable)?;
					let candidate = Candidate { cell, digit };
					let strat_res = Deduction::HiddenSingles(candidate, house.categorize());
					Self::push_new_candidate(&mut self.grid.state, &mut self.deduced_entries, candidate, &mut self.deductions, strat_res)?;

					// mark num as found
					singles.remove(digit.as_set());

					// everything in this house found
					// return to insert numbers immediately
					match stop_after_first {
						true => return Ok(()),
						false if singles.is_empty() => break, // continue next house
						_ => (), // find rest of singles in house
					}
				}
			}
		}
		Ok(())
	}

	// stop after first will only eliminate line OR field neighbors for ONE number
	// even if multiple are found at the same time
	pub(crate) fn find_locked_candidates(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
        self.update_cell_poss_house_solved()?;
		let (_, _, cell_poss_digits) = self.cell_poss_digits.get_mut();

		for chute in Chute::all() {
			let mut miniline_poss_digits: [Set<Digit>; 9] = [Set::NONE; 9];

			{ // compute possible digits for each miniline
			// TODO: switch to using house_solved_digits?
				let minilines = chute.minilines();
				for (&miniline, poss_digs) in minilines.iter().zip(miniline_poss_digits.iter_mut()) {
					for cell in miniline.cells() {
						*poss_digs |= cell_poss_digits[cell];
					}
				}
			}

			let mut line_unique_digits: [Set<Digit>; 3] = [Set::NONE; 3];
			let mut field_unique_digits: [Set<Digit>; 3] = [Set::NONE; 3];

			{
				let poss_digits = |chute_line, chute_field| miniline_poss_digits[ chute_line*3 + chute_field];
				for chute_line in 0..3 {
					let poss_digits_iter = (0..3)
						.map(|chute_field| poss_digits(chute_line, chute_field) );

					let (_, _, unique) = find_unique(poss_digits_iter);
					line_unique_digits[chute_line] = unique;
				}
				for chute_field in 0..3 {
					let poss_digits_iter = (0..3)
						.map(|chute_line| poss_digits(chute_line, chute_field) );;

					let (_, _, unique) = find_unique(poss_digits_iter);
					field_unique_digits[chute_field] = unique;
				}
			}

			// find minilines that contain the computed unique digits
			// remove them from the appropriate neighbors
			for (i, (&miniline, &poss_digits)) in chute.minilines().iter()
				.zip(miniline_poss_digits.iter())
				.enumerate()
			{
				let chute_line = i / 3;
				let chute_field = i % 3;

				let line_uniques =  poss_digits & line_unique_digits[chute_line];
				let field_uniques = poss_digits & field_unique_digits[chute_field];

				let (line_neighbors, field_neighbors) = miniline.neighbors();

				let eliminated_entries = &mut self.eliminated_entries;
				let mut find_impossibles = |uniques, neighbors: &[MiniLine; 2]| {
					let n_eliminated = eliminated_entries.len();
					for &neighbor in neighbors {
						let conflicts: Set<_> = miniline_poss_digits[neighbor.chute().as_index()] & uniques;
						if conflicts.is_empty() { continue }

						for cell in neighbor.cells() {
							let conflicts = cell_poss_digits[cell] & uniques;
							for digit in conflicts {
								eliminated_entries.push( Candidate { cell, digit } )
							}
						}
					}
					n_eliminated..eliminated_entries.len()
				};

				for &(uniques, neighbors) in [(line_uniques, &field_neighbors), (field_uniques, &line_neighbors)].iter()
					.filter(|&&(uniques, _)| !uniques.is_empty())
				{
					let rg_eliminations = find_impossibles(uniques, neighbors);
					if rg_eliminations.len() > 0 {
						// TODO: If stop_after_first is true, only enter the number whose conflicts were eliminated
						self.deductions.push(Deduction::LockedCandidates(miniline, uniques, rg_eliminations));

						if stop_after_first {
							return Ok(());
						}
					}
				}
			}
		}
		Ok(())
	}


	pub(crate) fn find_naked_subsets(&mut self, subset_size: u8, stop_after_first: bool) -> Result<(), Unsolvable> 	{
		fn walk_combinations(
			state: &mut StrategySolver,
			total_poss_digs: Set<Digit>,
			positions: SetIter<Position<House>>,
			house: House,
			stack: &mut Set<Position<House>>,
			subset_size: u8,
			stop_after_first: bool,
		) -> bool {
			// subsets of 5 and more numbers always have complementary subsets
			// of 9 - subset_size
			if stack.len() > subset_size { return false }
			if stack.len() == subset_size && total_poss_digs.len() == stack.len() {
				// found a subset
				let n_eliminated = state.eliminated_entries.len();
				for pos in !*stack {
					let cell = house.cell_at(pos);
				//for cell in house.cells().into_iter().filter(|cell| !stack.contains(cell)) {
					let conflicts = state.cell_poss_digits.state[cell] & total_poss_digs;
					for digit in conflicts {
						state.eliminated_entries.push(Candidate{ cell, digit });
					}
				}
				let rg_eliminations = n_eliminated..state.eliminated_entries.len();
				if rg_eliminations.len() > 0 {
					state.deductions.push(Deduction::NakedSubsets {
						house,
						positions: *stack,
						digits: total_poss_digs,
						conflicts: rg_eliminations
					});
					if stop_after_first {
						return true
					}
				}
			}

			let mut positions = positions;
			while let Some(position) = positions.next() {
				let pos_set = position.as_set();
				let cell = house.cell_at(position);
				let cell_poss_digits = state.cell_poss_digits.state[cell];
				// solved cell
				if cell_poss_digits.is_empty() { continue }
				*stack |= pos_set;
				let new_total_poss_digs = total_poss_digs | cell_poss_digits;

				// if true, then a subset was found and stop_after_first is set
				// stop recursion
				if walk_combinations(state, new_total_poss_digs, positions.clone(), house, stack, subset_size, stop_after_first) {
					return true
				};
				stack.remove(pos_set);
			}
			false
		}
		self.update_cell_poss_house_solved()?;

		let mut stack = Set::NONE;
		for house in House::all() {
			if self.house_solved_digits.state[house].is_full() { continue }
			// if true, then a subset was found and stop_after_first is set
			// stop looking
			if walk_combinations(self, Set::NONE, Set::ALL.into_iter(), house, &mut stack, subset_size, stop_after_first) {
				break
			};
		}
		Ok(())
	}

	pub(crate) fn find_hidden_subsets(&mut self, subset_size: u8, stop_after_first: bool) -> Result<(), Unsolvable> {
		fn walk_combinations(
			state: &mut StrategySolver,
			house: House,
			total_poss_pos: Set<Position<House>>,
			digits: SetIter<Digit>,
			stack: &mut Set<Digit>,
			subset_size: u8,
			stop_after_first: bool,
		) -> bool {
			if stack.len() > subset_size { return false }
			let house_poss_positions = state.house_poss_positions.state[house];

			if stack.len() == subset_size && total_poss_pos.len() == stack.len() {

				let n_eliminated = state.eliminated_entries.len();
				for digit in !*stack {
					let conflicts = house_poss_positions[digit] & total_poss_pos;
					for pos in conflicts {
						let cell = house.cell_at(pos);
						state.eliminated_entries.push(Candidate{ cell, digit });
					}
				}
				let rg_eliminations = n_eliminated..state.eliminated_entries.len();
				if rg_eliminations.len() > 0 {
					state.deductions.push(Deduction::HiddenSubsets {
						house,
						digits: stack.clone(),
						positions: total_poss_pos,
						conflicts: rg_eliminations
					});
					if stop_after_first {
						return true
					}
				}
			}

			let mut digits = digits;
			while let Some(digit) = digits.next() {
				let digit_set = digit.as_set();
				let num_poss_pos = house_poss_positions[digit];
				// solved cell
				if num_poss_pos.is_empty() { continue }
				*stack |= digit_set;
				let new_total_poss_pos = total_poss_pos | num_poss_pos;
				if walk_combinations(state, house, new_total_poss_pos, digits.clone(), stack, subset_size, stop_after_first) {
					return true
				};
				stack.remove(digit_set);
			}
			false
		}

		self.update_house_poss_positions()?;

		let mut stack = Set::NONE;
		for house in House::all() {
			if self.house_solved_digits.state[house].is_full() { continue }
			let digits = Set::ALL;
			if walk_combinations(self, house, Set::NONE, digits.into_iter(), &mut stack, subset_size, stop_after_first) {
				break
			};
		}
		Ok(())
	}

	pub(crate) fn find_xwings(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
		self.find_fish(2, stop_after_first)
	}


	pub(crate) fn find_swordfish(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
		self.find_fish(3, stop_after_first)
	}


	pub(crate) fn find_jellyfish(&mut self, stop_after_first: bool) -> Result<(), Unsolvable> {
		self.find_fish(4, stop_after_first)
	}

	fn find_fish(&mut self, max_size: u8, stop_after_first: bool) -> Result<(), Unsolvable> {
		self.update_house_poss_positions().unwrap(); // TODO: why is there an unwrap here?
		self.update_cell_poss_house_solved()?;
		let mut stack = Set::NONE;
		for digit in (1..10).map(Digit::new) {
			// 0..9 = rows, 9..18 = cols
			for &lines in &[Line::ALL_ROWS, Line::ALL_COLS] {
				if basic_fish_walk_combinations(self, digit, max_size, &mut stack, lines.into_iter(), lines, Set::NONE, stop_after_first) {
					return Ok(());
				};
			}
		}
		Ok(())
	}

	pub(crate) fn find_singles_chain(&mut self) -> Result<(), Unsolvable> {
        #[derive(Copy, Clone, PartialEq, Eq)]
        enum Colour {
            Uncoloured,
            A,
            B,
        }

        fn follow_links(digit: Digit, cell: Cell, is_a: bool, sudoku: &StrategySolver, cell_color: &mut CellArray<Colour>, link_nr: u8, cell_linked: &mut CellArray<u8>) {
            if cell_linked[cell] <= link_nr { return }

            for &(con_house, current_pos) in &[
                (cell.row().house(), cell.row_pos()),
                (cell.col().house(), cell.col_pos()),
                (cell.block().house(), cell.block_pos()),
            ] {
                let house_poss_positions = sudoku.house_poss_positions.state[con_house][digit];
                if house_poss_positions.len() == 2 {
                    let other_pos = house_poss_positions.without(current_pos.as_set()).one_possibility();
                    let other_cell = con_house.cell_at(other_pos);

                    match cell_linked[other_cell] <= link_nr {
                        true => continue,
                        false => cell_linked[other_cell] = link_nr,
                    };

                    cell_color[other_cell] = if is_a { Colour::A } else { Colour::B };

                    follow_links(digit, other_cell, !is_a, sudoku, cell_color, link_nr, cell_linked);
                }
            }
        }

        for digit in Set::<Digit>::ALL {
            let mut cell_touched = [false; N_CELLS];
            let mut link_nr = 0;

            let mut cell_linked = CellArray([0; 81]);
            let mut cell_color = CellArray([Colour::Uncoloured; 81]);

            for house in House::all() {
                let house_poss_positions = self.house_poss_positions.state[house][digit];
                if house_poss_positions.len() == 2 {
                    let first = house_poss_positions.one_possibility();
                    let cell = house.cell_at(first);

                    match cell_touched[cell.as_index()] {
                        true => continue,
                        false => cell_touched[cell.as_index()] = true,
                    };

                    follow_links(digit, cell, true, self, &mut cell_color, link_nr, &mut cell_linked);
                    link_nr += 1;
                }
            }

            for link_nr in 0..link_nr {
                // Rule 1:
                // if two cells in the same row, part of the same chain
                // have the same color, those cells must not contain the number
                // Rule 2:
                // if one cell is neighbor to two cells with opposite colours
                // it can not contain the number


                // ===== Rule 1 ======
                for house in House::all() {
                    // Collect colours in this link chain and this house
                    let mut house_colors = [Colour::Uncoloured; 9];
                    for (pos, cell) in house.cells()
                        .into_iter()
                        .enumerate()
                        // TODO: Double check the logic here
                        // this used to take the pos for indexing
                        .filter(|&(_, cell)| cell_linked[cell] == link_nr)
                    {
                        house_colors[pos] = cell_color[cell];
                    }

                    let (n_a, n_b) = house_colors.iter()
                        .fold((0, 0), |(n_a, n_b), &colour| {
                            match colour {
                                Colour::A => (n_a+1, n_b),
                                Colour::B => (n_a, n_b+1),
                                Colour::Uncoloured => (n_a, n_b),
                            }
                        });

                    fn mark_impossible(digit: Digit, link_nr: u8, colour: Colour, cell_color: CellArray<Colour>, cell_linked: CellArray<u8>, impossible_entries: &mut Vec<Candidate>) {
                        Cell::all().zip(cell_color.iter()).zip(cell_linked.iter())
                            .filter(|&((_, &cell_colour), &cell_link_nr)| link_nr == cell_link_nr && colour == cell_colour)
                            .for_each(|((cell, _), _)| impossible_entries.push( Candidate { cell, digit }));
                    }

                    let impossible_colour;
                    match (n_a >= 2, n_b >= 2) {
                        (true, true) => return Err(Unsolvable),
                        (true, false) => impossible_colour = Colour::A,
                        (false, true) => impossible_colour = Colour::B,
                        (false, false) => continue,
                    };
                    mark_impossible(digit, link_nr, impossible_colour, cell_color, cell_linked, &mut self.eliminated_entries);
                    // chain handled, go to next
                    // note: as this eagerly marks a colour impossible as soon as a double in any colour is found
                    //       a case of two doubles in some later house will not always be found
                    //       impossibility is then detected further down the strategy chain
                    break
                }

                // ===== Rule 2 =====
                let mut cell_sees_colour = CellArray([(false, false); 81]);
                for ((cell, &cell_colour), _) in Cell::all()
					.zip(cell_color.iter())
                    .zip(cell_linked.iter())
                    .filter(|&((_, &cell_colour), &cell_link_nr)| link_nr == cell_link_nr && cell_colour != Colour::Uncoloured)
                {
                    for &house in &cell.houses() {
                        for neighbor_cell in house.cells().into_iter().filter(|&c| cell != c) {
                            let (sees_a, sees_b) = cell_sees_colour[neighbor_cell];
                            if cell_colour == Colour::A && !sees_a {
                                cell_sees_colour[neighbor_cell].0 = true;
                                if sees_b {
                                    self.eliminated_entries.push( Candidate{ cell: neighbor_cell, digit })
                                }
                            } else if cell_colour == Colour::B && !sees_b {
                                cell_sees_colour[neighbor_cell].1 = true;
                                if sees_a {
                                    self.eliminated_entries.push( Candidate{ cell: neighbor_cell, digit })
                                }
                            }
                        }
                    }
                }
            }
        }
		Ok(())
	}
}

//             goal_depth
// <degenerated>   1 (basically a naked/hidden single, not supported by this fn)
// x-wing          2
// swordfish       3
// jellyfish       4
fn basic_fish_walk_combinations(
	sudoku: &mut StrategySolver,
	digit: Digit,
	goal_depth: u8,
	stack: &mut Set<Line>,
	lines: SetIter<Line>,
	all_lines: Set<Line>,
	union_poss_pos: Set<Position<Line>>,
	stop_after_first: bool,
) -> bool {
	if stack.len() == goal_depth {
		// nothing of interest found
		if union_poss_pos.len() != goal_depth as u8 { return false }
		// found xwing, swordfish, jellyfish, whatever-the-name
		let n_eliminated = sudoku.eliminated_entries.len();
		for line in all_lines.without(*stack) {
			for pos in union_poss_pos {
				let cell = line.cell_at(pos);
				let cell_mask = sudoku.cell_poss_digits.state[cell];
				if cell_mask.overlaps(digit.as_set()) {
					sudoku.eliminated_entries.push(Candidate{ cell, digit });
				}
			}
		}

		let rg_eliminations = n_eliminated..sudoku.eliminated_entries.len();
		if rg_eliminations.len() > 0 {
			let lines = stack.clone();
			let positions = union_poss_pos;
			let conflicts = rg_eliminations;

			sudoku.deductions.push(
				Deduction::BasicFish {
					lines, digit, conflicts, positions,
				}
			);
			if stop_after_first {
				return true
			}
		}
	}

	let mut lines = lines;
	while let Some(line) = lines.next() {
		let line_set = line.as_set();
		let possible_pos = sudoku.house_poss_positions.state[line][digit];
		let n_poss = possible_pos.len();
		let new_union_poss_pos = union_poss_pos | possible_pos.as_line_set(); // TODO: remove the "cast"

		// n_poss == 0 => solved row (or impossible)
		// n_poss == 1 => hidden single
		if n_poss < 2 || new_union_poss_pos.len() > goal_depth as u8 { continue }
		*stack |= line_set;
		if basic_fish_walk_combinations(sudoku, digit, goal_depth, stack, lines.clone(), all_lines, new_union_poss_pos, stop_after_first) {
			return true
		};
		stack.remove(line_set);
	}
	false
}


#[derive(Debug, Clone)]
pub(crate) struct State<T> {
	next_deduced: u16,
	last_eliminated: u16, // probably doesn't exceed 2^8, but can't prove it
	state: T,
}

impl<T> State<T> {
	fn from(this: T) -> Self {
		State {
			next_deduced: 0,
			last_eliminated: 0,
			state: this,
		}
	}
}

impl<T> State<T> {
	fn get_mut(&mut self) -> (&mut u16, &mut u16, &mut T) {
		let State { next_deduced: ld, last_eliminated: le, state } = self;
		(ld, le, state)
	}
}

#[inline]
fn find_unique<I: Iterator<Item=Set<Digit>>>(possibilities: I) -> (Set<Digit>, Set<Digit>, Set<Digit>) {
	let mut unsolved = Set::NONE;
	let mut multiple_unsolved = Set::NONE;

	for poss_digits in possibilities {
		multiple_unsolved |= unsolved & poss_digits;
		unsolved |= poss_digits;
	}
	// >= 1, >1, =1 occurences
	(unsolved, multiple_unsolved, unsolved.without(multiple_unsolved) )
}

#[cfg(test)]
mod test {
    use super::*;
    fn read_sudokus(sudokus_str: &str) -> Vec<Sudoku> {
    sudokus_str.lines()
            .map(|line| Sudoku::from_str_line(line).unwrap_or_else(|err| panic!("{:?}", err)))
            .collect()
    }

    fn strategy_solver_correct_solution<F>(sudokus: Vec<Sudoku>, solved_sudokus: Vec<Sudoku>, solver: F)
        where F: Fn(StrategySolver, &[Strategy]) -> Result<(Sudoku, Deductions), (Sudoku, Deductions)>,
    {
        let n_sudokus = sudokus.len();
        let strategies = Strategy::ALL;
        let mut unsolved = vec![];
        for (i, (sudoku, solved_sudoku)) in sudokus.into_iter().zip(solved_sudokus).enumerate() {
            let cache = StrategySolver::from_sudoku(sudoku);
            match solver(cache, &strategies) {
                Ok((solution, _deductions)) => assert_eq!(solution, solved_sudoku),
                Err((part_solved, _deductions)) => unsolved.push((i, sudoku, part_solved, solved_sudoku)),
            }
        }
        if unsolved.len() != 0 {
            println!("Could not solve {}/{} sudokus:\n", unsolved.len(), n_sudokus);


            for (i, sudoku, part_solution, _solution) in unsolved {
            	println!("\nsudoku nr {}:\n{}\n{}\n{}", i+1, sudoku.to_str_line(), part_solution.to_str_line(), _solution.to_str_line());
            }
            panic!();
        }
    }

    #[test]
    fn strategy_solver_correct_solution_easy_sudokus() {
        let sudokus = read_sudokus( include_str!("../../sudokus/Lines/easy_sudokus.txt") );
        let solved_sudokus = read_sudokus( include_str!("../../sudokus/Lines/solved_easy_sudokus.txt") );
        strategy_solver_correct_solution(sudokus, solved_sudokus, StrategySolver::solve);
    }

    #[test]
    fn strategy_solver_correct_solution_medium_sudokus() {
		// the 9th sudoku requires more advanced strategies
		let filter_9 = |vec: Vec<_>| vec.into_iter()
			.enumerate()
			.filter(|&(i, _)| i != 8)
			.map(|(_, sudoku)| sudoku)
			.collect::<Vec<_>>();
        let sudokus = filter_9(read_sudokus( include_str!("../../sudokus/Lines/medium_sudokus.txt") ));
        let solved_sudokus = filter_9(read_sudokus( include_str!("../../sudokus/Lines/solved_medium_sudokus.txt") ));
        strategy_solver_correct_solution(sudokus, solved_sudokus, StrategySolver::solve);
    }
}