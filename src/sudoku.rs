use consts::*;
use positions::*;
use types::{Mask, Digit, Array81, Entry, ParseError, Unsolvable};

use std::{fmt, slice, iter};
use std::io::BufRead;

/// The main structure exposing all the functionality of the library
#[derive(Copy)]
pub struct Sudoku([u8; 81]);

impl PartialEq for Sudoku {
	fn eq(&self, other: &Sudoku) -> bool {
		&self.0[..] == &other.0[..]
	}
}

impl Eq for Sudoku {}

impl fmt::Debug for Sudoku {
	fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		self.0.fmt(fmt)
	}
}

impl Clone for Sudoku {
	fn clone(&self) -> Self {
		*self
	}
}

pub type Iter<'a> = iter::Map<slice::Iter<'a, u8>, fn(&u8)->Option<u8>>; // Iter over Sudoku cells

impl Sudoku {
	/// Creates a new sudoku based on a `&str`. See the crate documentation
	/// for an example of the expected format
	pub fn from_str(s: &str) -> Result<Sudoku, ParseError> {
		Sudoku::from_reader(s.as_bytes())
	}

	/// Creates a new sudoku based on a reader. See the crate documentation
	/// for an example of the expected format
	pub fn from_reader<T: BufRead>(reader: T) -> Result<Sudoku, ParseError> {
		let mut grid = [0; N_CELLS];

		// Read a row per line
		let mut line_count = 0;
		for (line_nr, line) in Iterator::zip(1..9+1, reader.lines().take(9)) {
			line_count += 1;
			let line = line.ok().unwrap_or("".to_string());
			let trimmed_line = line.trim_right();
			if trimmed_line.chars().filter(|&c| c!= '|').count() != 9 {
				return Err(ParseError::InvalidLineLength(line_nr));
			}

			for (col, ch) in trimmed_line.chars().filter(|&c| c != '|').enumerate() {
				match ch {
					'1'...'9' => grid[(line_nr-1) as usize *9 + col] = ch.to_digit(10).unwrap() as u8,
					'_'       => grid[(line_nr-1) as usize *9 + col] = 0,
					_         => return Err(ParseError::InvalidNumber(line_nr, ch)),
				}
			}
		}

		if line_count < 9 {
			Err(ParseError::NotEnoughRows)
		} else {
			Ok(Sudoku(grid))
		}
	}

    fn into_solver(self) -> Result<SudokuSolver, Unsolvable> {
        SudokuSolver::from_sudoku(self)
    }

	/// Try to find a solution to the sudoku and fill it in. Return true if a solution was found.
	/// This is a convenience interface. Use one of the other solver methods for better error handling
	pub fn solve(&mut self) -> bool {
		match self.clone().into_solver().map(|solver| solver.solve_one()).unwrap_or(None) {
			Some(solution) => {
				*self = solution;
				true
			},
			None => false,
		}
	}

	/// Find a solution to the sudoku. If multiple solutions exist, it will not find them and just stop at the first.
	/// Return `None` if no solution exists.
    pub fn solve_one(self) -> Option<Sudoku> {
        self.into_solver().map(SudokuSolver::solve_one).unwrap_or(None)
    }

    /// Solve sudoku and return solution if solution is unique.
	pub fn solve_unique(self) -> Option<Sudoku> {
		self.into_solver().map(SudokuSolver::solve_unique).unwrap_or(None)
	}

	/// Solve sudoku and return the first `limit` solutions it finds. If less solutions exist, return only those. Return `None` if no solution exists.
	/// No specific ordering of solutions is promised. It can change across versions.
    pub fn solve_at_most(self, limit: usize) -> Option<Vec<Sudoku>> {
        let results = self.into_solver().map(|solver| solver.solve_at_most(limit))
			.unwrap_or(vec![]);
		if results.len() == 0 {
			None
		} else {
			Some(results)
		}
    }

	/// Check whether the sudoku is solved.
	pub fn is_solved(&self) -> bool {
		self.clone().into_solver().map(|solver| solver.is_solved()).unwrap_or(false)
	}

    /// Returns an Iterator over sudoku, going from left to right, top to bottom
    pub fn iter(&self) -> Iter {
        self.0.iter().map(num_to_opt)
    }
}

fn num_to_opt(num: &u8) -> Option<u8> {
	if *num == 0 { None } else { Some(*num) }
}

impl fmt::Display for Sudoku {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for entry in self.0.iter().enumerate().map(|(cell, &num)| Entry { cell: cell as u8, num: num } ) {
			try!( match (entry.row(), entry.col()) {
				(_, 3) | (_, 6) => write!(f, " "),    // seperate fields in columns
				(3, 0) | (6, 0) => write!(f, "\n\n"), // separate fields in rows
				(_, 0)          => write!(f, "\n"),   // separate lines not between fields
				_ => Ok(()),
			});
			//try!(
            try!( match entry.num() {
                0 => write!(f, "_"),
                1...9 => write!(f, "{}", entry.num()),
                _ => unreachable!(),
            });
                //uwrite!(f, "{}", entry.num())
            //);
		}
		Ok(())
	}
}

// Solving happens by an exact cover algorithm
// There are a total of 729 (81 cells * 9 numbers) sudoku entry possibilities
//
// Every entry (cell-number-combination) satisfies 4 constraints
// 1. a row    needs to have 1 of each number (9 rows, 9 numbers each)
// 2. a column needs to have 1 of each number (9 cols, 9 numbers each)
// 3. a field  needs to have 1 of each number (9 fields, 9 numbers each)
// 4. a cell needs to be filled               (81 cells, 1 number each)
//
// For a total of 81*4 = 324 constraints
//
// The covers property in SudokuSolver contains the information what entries can
// be added at a certain point in the solving process, which constraints are
// already satisfied and how many possibilities still exist for a given constraint.
// See also the covers module.
//
// Solving happens by recursively walking the tree of possible sudokus
// If some constraint can only be satisfied by 1 entry, it will be entered immediately
// This is equivalent to finding naked singles and hidden singles
// If no entry can be deduced, a constraint with the least amount of possibilites
// is chosen and all possibilites tried out.

// Helper struct for recursive solving
#[derive(Clone, Debug)]
pub struct SudokuSolver {
	pub grid: Sudoku,
	pub n_solved_cells: u8,
	pub cell_poss_digits: Array81<Mask<Digit>>,
	pub zone_solved_digits: [Mask<Digit>; 27],
}

impl SudokuSolver {
	fn new() -> SudokuSolver {
		SudokuSolver {
			grid: Sudoku([0; 81]),
			n_solved_cells: 0,
			cell_poss_digits: Array81([Mask::all(); 81]),
			zone_solved_digits: [Mask::none(); 27],
		}
	}

	pub fn from_sudoku(sudoku: Sudoku) -> Result<SudokuSolver, Unsolvable> {
		let mut solver = Self::new();
		let mut stack = sudoku.iter()
			.enumerate()
			.flat_map(|(i, num)| num.map(|n| Entry { cell: i as u8, num: n }))
			.collect();
		solver.insert_entries(&mut stack)?;
		Ok(solver)
	}

	fn _insert_entry(&mut self, entry: Entry) {
		self.n_solved_cells += 1;
		self.grid.0[entry.cell()] = entry.num;
		self.cell_poss_digits[entry.cell()] = Mask::none();
		self.zone_solved_digits[entry.row() as usize +ROW_OFFSET] |= entry.mask();
		self.zone_solved_digits[entry.col() as usize +COL_OFFSET] |= entry.mask();
		self.zone_solved_digits[entry.field() as usize +FIELD_OFFSET] |= entry.mask();
	}
/*
	fn decrement_possibilities_count(&mut self, impossible_entry: Entry) {
		self.covers.possibilities_count[impossible_entry.row_constraint()] -= 1;
		self.covers.possibilities_count[impossible_entry.col_constraint()] -= 1;
		self.covers.possibilities_count[impossible_entry.field_constraint()] -= 1;
		self.covers.possibilities_count[impossible_entry.cell_constraint()] -= 1;
	}
*/
/*
	fn insert_entry(&mut self, entry: Entry) -> Result<(), Unsolvable> {
		self._insert_entry(entry)?;

		// remove impossible entries, keep possibilities counter accurate
		let mut entries = mem::replace(&mut self.covers.entries, vec![] );
		entries.retain(|&old_entry| {
			if old_entry.conflicts_with(entry) { // remove old_entry
				self.decrement_possibilities_count(old_entry);
				false
			} else {
				true
			}
		});
		self.covers.entries = entries;
		Ok(())
	}
*/
	fn insert_entries(&mut self, stack: &mut Vec<Entry>) -> Result<(), Unsolvable> {
		for entry in stack.drain(..) {
			// cell already solved from previous entry in stack, skip
			if self.cell_poss_digits[entry.cell()] == Mask::none() { continue }

			// is entry still possible?
			// have to check zone possibilities, because cell possibility
			// is temporarily out of date
			if self.zone_solved_digits[entry.row() as usize + ROW_OFFSET] & entry.mask() != Mask::none()
			|| self.zone_solved_digits[entry.col() as usize + COL_OFFSET] & entry.mask() != Mask::none()
			|| self.zone_solved_digits[entry.field() as usize +FIELD_OFFSET] & entry.mask() != Mask::none()
			{
				return Err(Unsolvable);
			}

			self._insert_entry(entry);
		}

		// update cell possibilities from zone masks
		for cell in 0..81 {
			let cell_mask = &mut self.cell_poss_digits[cell as usize];
			if *cell_mask == Mask::none() { continue }
			let zones_mask = self.zone_solved_digits[row_zone(cell)]
				| self.zone_solved_digits[col_zone(cell)]
				| self.zone_solved_digits[field_zone(cell)];

			*cell_mask &= !zones_mask;
			if let Some(num) = cell_mask.unique_num()? {
				stack.push(Entry{ cell: cell as u8, num });
			}
		}
		Ok(())
	}

	#[inline]
	pub fn is_solved(&self) -> bool {
		self.n_solved_cells == 81
	}
/*
	#[inline]
	fn is_impossible(&self) -> bool {
		Iterator::zip( self.covers.possibilities_count.iter(), self.covers.covered.iter() )
			.any(|(&poss, &covered)| !covered && poss == 0)
	}
*/

	fn find_hidden_singles(&mut self, stack: &mut Vec<Entry>) -> Result<(), Unsolvable> {
		if let Some(res) = (0..27).map(|zone| {
				let mut unsolved = Mask::none();
				let mut multiple_unsolved = Mask::none();

				let cells = cells_of_zone(zone);
				for &cell in cells.iter() {
					let poss_digits = self.cell_poss_digits[cell as usize];
					multiple_unsolved |= unsolved & poss_digits;
					unsolved |= poss_digits;
				}
				if unsolved | self.zone_solved_digits[zone as usize] != Mask::all() {
					return Err(Unsolvable);
				}

				Ok((unsolved & !multiple_unsolved, cells))
			})
			.find(|res| res.as_ref()
				.map(|&(m, _)| m != Mask::none())
				.unwrap_or(true)
			)
		{
			let (singles, cells) = res?;
			for &cell in cells.iter() {
				let mask = self.cell_poss_digits[cell as usize];
				if mask & singles != Mask::none() {
					let num = (mask & singles).unique_num().expect("unexpected empty mask").ok_or(Unsolvable)?;
					stack.push(Entry{cell, num} );
				}
			}
		}
		Ok(())
	}

	fn find_good_guess(&mut self) -> Entry {
		let mut min_possibilities = 10;
		let mut best_cell = 100;

		for cell in 0..81 {
			let cell_mask = self.cell_poss_digits[cell as usize];
			let n_possibilities = cell_mask.n_possibilities();
			// 0 means cell was already processed or its impossible in which case,
			// it should have been caught elsewhere
			// 1 shouldn't happen for the same reason, should have been processed
			if n_possibilities > 0 && n_possibilities < min_possibilities {
				best_cell = cell;
				min_possibilities = n_possibilities;
				if n_possibilities == 2 { break }
			}
		}

		let num = self.cell_poss_digits[best_cell as usize].one_possibility();
		Entry{ num, cell: best_cell }
	}

	// remove impossible digits from masks for given cell
	// also check for naked singles and impossibility of sudoku
	fn remove_impossibilities(&mut self, cell: u8, impossible: Mask<Digit>, stack: &mut Vec<Entry>) -> Result<(), Unsolvable> {
		let cell_mask = &mut self.cell_poss_digits[cell as usize];
		*cell_mask &= !impossible;
		if let Some(num) = cell_mask.unique_num()? {
			stack.push(Entry{ cell, num });
		}
		Ok(())
	}

	/*
	// may fail, but only if used incorrectly
	#[inline]
	fn matching_entry(&self, constraint_nr: usize) -> Entry {
		self.covers.entries.iter()
			.cloned()
			.find(|e| e.constrains(constraint_nr))
			.unwrap()
	}
	*/
	pub fn solve_one(self) -> Option<Sudoku> {
		self.solve_at_most(1)
			.into_iter()
			.next()
	}

	pub fn solve_unique(self) -> Option<Sudoku> {
		let result = self.solve_at_most(2);
		if result.len() == 1 {
			result.into_iter().next()
		} else {
			None
		}
	}

	pub fn solve_at_most(self, limit: usize) -> Vec<Sudoku> {
		let mut solutions = vec![];
		let mut stack = Vec::with_capacity(81);
		let _ = self._solve_at_most(limit, &mut stack, &mut solutions);
		solutions
	}

	fn _solve_at_most(mut self, limit: usize, stack: &mut Vec<Entry>, solutions: &mut Vec<Sudoku>) -> Result<(), Unsolvable> {
		self.insert_entries(stack)?;
		if self.is_solved() {
			solutions.push(self.grid.clone());
			return Ok(())
		}

		self.find_hidden_singles(stack)?;
		if !stack.is_empty() {
			return self._solve_at_most(limit, stack, solutions);
		}

		let entry = self.find_good_guess();
		stack.push(entry);
		let _ = self.clone()._solve_at_most(limit, stack, solutions);
		stack.clear();
		if solutions.len() == limit { return Ok(()) }

		self.remove_impossibilities(entry.cell, entry.mask(), stack)?;
		self._solve_at_most(limit, stack, solutions)
	}
}