// Copyright 2018 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Copyright (c) 2015-2017 Ivo Wetzel
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

//! Adapted from https://github.com/behnam/rust-cursive-table-view
//! A basic table view implementation for [cursive](https://crates.io/crates/cursive).

#![deny(
	missing_docs, missing_copy_implementations, trivial_casts, trivial_numeric_casts, unsafe_code,
	unused_import_braces, unused_qualifications
)]

// Crate Dependencies ---------------------------------------------------------
extern crate cursive;

// STD Dependencies -----------------------------------------------------------
use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

// External Dependencies ------------------------------------------------------
use cursive::align::HAlign;
use cursive::direction::Direction;
use cursive::event::{Callback, Event, EventResult, Key};
use cursive::theme::ColorStyle;
use cursive::theme::PaletteColor::{Highlight, HighlightInactive, Primary};
use cursive::vec::Vec2;
use cursive::view::{ScrollBase, View};
use cursive::With;
use cursive::{Cursive, Printer};

/// A trait for displaying and sorting items inside a
/// [`TableView`](struct.TableView.html).
pub trait TableViewItem<H>: Clone + Sized
where
	H: Eq + Hash + Copy + Clone + 'static,
{
	/// Method returning a string representation of the item for the
	/// specified column from type `H`.
	fn to_column(&self, column: H) -> String;

	/// Method comparing two items via their specified column from type `H`.
	fn cmp(&self, other: &Self, column: H) -> Ordering
	where
		Self: Sized;
}

/// View to select an item among a list, supporting multiple columns for
/// sorting.
///
/// # Examples
///
/// ```rust
/// # extern crate cursive;
/// # extern crate cursive_table_view;
/// # use std::cmp::Ordering;
/// # use cursive_table_view::{TableView, TableViewItem};
/// # use cursive::align::HAlign;
/// # fn main() {
/// // Provide a type for the table's columns
/// #[derive(Copy, Clone, PartialEq, Eq, Hash)]
/// enum BasicColumn {
///     Name,
///     Count,
///     Rate
/// }
///
/// // Define the item type
/// #[derive(Clone, Debug)]
/// struct Foo {
///     name: String,
///     count: usize,
///     rate: usize
/// }
///
/// impl TableViewItem<BasicColumn> for Foo {
///
///     fn to_column(&self, column: BasicColumn) -> String {
///         match column {
///             BasicColumn::Name => self.name.to_string(),
///             BasicColumn::Count => format!("{}", self.count),
///             BasicColumn::Rate => format!("{}", self.rate)
///         }
///     }
///
///     fn cmp(&self, other: &Self, column: BasicColumn) -> Ordering where Self: Sized {
///         match column {
///             BasicColumn::Name => self.name.cmp(&other.name),
///             BasicColumn::Count => self.count.cmp(&other.count),
///             BasicColumn::Rate => self.rate.cmp(&other.rate)
///         }
///     }
///
/// }
///
/// // Configure the actual table
/// let table = TableView::<Foo, BasicColumn>::new()
///                      .column(BasicColumn::Name, "Name", |c| c.width(20))
///                      .column(BasicColumn::Count, "Count", |c| c.align(HAlign::Center))
///                      .column(BasicColumn::Rate, "Rate", |c| {
///                          c.ordering(Ordering::Greater).align(HAlign::Right).width(20)
///                      })
///                      .default_column(BasicColumn::Name);
/// # }
/// ```
pub struct TableView<T: TableViewItem<H>, H: Eq + Hash + Copy + Clone + 'static> {
	enabled: bool,
	scrollbase: ScrollBase,
	last_size: Vec2,

	column_select: bool,
	columns: Vec<TableColumn<H>>,
	column_indices: HashMap<H, usize>,

	focus: usize,
	items: Vec<T>,
	rows_to_items: Vec<usize>,

	on_sort: Option<Rc<Fn(&mut Cursive, H, Ordering)>>,
	// TODO Pass drawing offsets into the handlers so a popup menu
	// can be created easily?
	on_submit: Option<Rc<Fn(&mut Cursive, usize, usize)>>,
	on_select: Option<Rc<Fn(&mut Cursive, usize, usize)>>,
}

impl<T: TableViewItem<H>, H: Eq + Hash + Copy + Clone + 'static> TableView<T, H> {
	/// Creates a new empty `TableView` without any columns.
	///
	/// A TableView should be accompanied by a enum of type `H` representing
	/// the table columns.
	pub fn new() -> Self {
		Self {
			enabled: true,
			scrollbase: ScrollBase::new(),
			last_size: Vec2::new(0, 0),

			column_select: false,
			columns: Vec::new(),
			column_indices: HashMap::new(),

			focus: 0,
			items: Vec::new(),
			rows_to_items: Vec::new(),

			on_sort: None,
			on_submit: None,
			on_select: None,
		}
	}

	/// Adds a column for the specified table column from type `H` along with
	/// a title for its visual display.
	///
	/// The provided callback can be used to further configure the
	/// created [`TableColumn`](struct.TableColumn.html).
	pub fn column<S: Into<String>, C: FnOnce(TableColumn<H>) -> TableColumn<H>>(
		mut self,
		column: H,
		title: S,
		callback: C,
	) -> Self {
		self.column_indices.insert(column, self.columns.len());
		self.columns
			.push(callback(TableColumn::new(column, title.into())));

		// Make the first column the default one
		if self.columns.len() == 1 {
			self.default_column(column)
		} else {
			self
		}
	}

	/// Sets the initially active column of the table.
	pub fn default_column(mut self, column: H) -> Self {
		if self.column_indices.contains_key(&column) {
			for c in &mut self.columns {
				c.selected = c.column == column;
				if c.selected {
					c.order = c.default_order;
				} else {
					c.order = Ordering::Equal;
				}
			}
		}
		self
	}

	/// Sorts the table using the specified table `column` and the passed
	/// `order`.
	pub fn sort_by(&mut self, column: H, order: Ordering) {
		if self.column_indices.contains_key(&column) {
			for c in &mut self.columns {
				c.selected = c.column == column;
				if c.selected {
					c.order = order;
				} else {
					c.order = Ordering::Equal;
				}
			}
		}

		self.sort_items(column, order);
	}

	/// Sorts the table using the currently active column and its
	/// ordering.
	pub fn sort(&mut self) {
		if let Some((column, order)) = self.order() {
			self.sort_items(column, order);
		}
	}

	/// Returns the currently active column that is used for sorting
	/// along with its ordering.
	///
	/// Might return `None` if there are currently no items in the table
	/// and it has not been sorted yet.
	pub fn order(&self) -> Option<(H, Ordering)> {
		for c in &self.columns {
			if c.order != Ordering::Equal {
				return Some((c.column, c.order));
			}
		}
		None
	}

	/// Disables this view.
	///
	/// A disabled view cannot be selected.
	pub fn disable(&mut self) {
		self.enabled = false;
	}

	/// Re-enables this view.
	pub fn enable(&mut self) {
		self.enabled = true;
	}

	/// Enable or disable this view.
	pub fn set_enabled(&mut self, enabled: bool) {
		self.enabled = enabled;
	}

	/// Returns `true` if this view is enabled.
	pub fn is_enabled(&self) -> bool {
		self.enabled
	}

	/// Sets a callback to be used when a selected column is sorted by
	/// pressing `<Enter>`.
	///
	/// # Example
	///
	/// ```norun
	/// rt(|siv: &mut Cursive, column: BasicColumn, order: Ordering| {});
	/// ```
	pub fn set_on_sort<F>(&mut self, cb: F)
	where
		F: Fn(&mut Cursive, H, Ordering) + 'static,
	{
		self.on_sort = Some(Rc::new(move |s, h, o| cb(s, h, o)));
	}

	/// Sets a callback to be used when a selected column is sorted by
	/// pressing `<Enter>`.
	///
	/// Chainable variant.
	///
	/// # Example
	///
	/// ```norun
	/// siv: &mut Cursive, column: BasicColumn, order: Ordering| {});
	/// ```
	pub fn on_sort<F>(self, cb: F) -> Self
	where
		F: Fn(&mut Cursive, H, Ordering) + 'static,
	{
		self.with(|t| t.set_on_sort(cb))
	}

	/// Sets a callback to be used when `<Enter>` is pressed while an item
	/// is selected.
	///
	/// Both the currently selected row and the index of the corresponding item
	/// within the underlying storage vector will be given to the callback.
	///
	/// # Example
	///
	/// ```norun
	/// bmit(|siv: &mut Cursive, row: usize, index: usize| {});
	/// ```
	pub fn set_on_submit<F>(&mut self, cb: F)
	where
		F: Fn(&mut Cursive, usize, usize) + 'static,
	{
		self.on_submit = Some(Rc::new(move |s, row, index| cb(s, row, index)));
	}

	/// Sets a callback to be used when `<Enter>` is pressed while an item
	/// is selected.
	///
	/// Both the currently selected row and the index of the corresponding item
	/// within the underlying storage vector will be given to the callback.
	///
	/// Chainable variant.
	///
	/// # Example
	///
	/// ```norun
	/// (|siv: &mut Cursive, row: usize, index: usize| {});
	/// ```
	pub fn on_submit<F>(self, cb: F) -> Self
	where
		F: Fn(&mut Cursive, usize, usize) + 'static,
	{
		self.with(|t| t.set_on_submit(cb))
	}

	/// Sets a callback to be used when an item is selected.
	///
	/// Both the currently selected row and the index of the corresponding item
	/// within the underlying storage vector will be given to the callback.
	///
	/// # Example
	///
	/// ```norun
	/// lect(|siv: &mut Cursive, row: usize, index: usize| {});
	/// ```
	pub fn set_on_select<F>(&mut self, cb: F)
	where
		F: Fn(&mut Cursive, usize, usize) + 'static,
	{
		self.on_select = Some(Rc::new(move |s, row, index| cb(s, row, index)));
	}

	/// Sets a callback to be used when an item is selected.
	///
	/// Both the currently selected row and the index of the corresponding item
	/// within the underlying storage vector will be given to the callback.
	///
	/// Chainable variant.
	///
	/// # Example
	///
	/// ```norun
	/// (|siv: &mut Cursive, row: usize, index: usize| {});
	/// ```
	pub fn on_select<F>(self, cb: F) -> Self
	where
		F: Fn(&mut Cursive, usize, usize) + 'static,
	{
		self.with(|t| t.set_on_select(cb))
	}

	/// Removes all items from this view.
	pub fn clear(&mut self) {
		self.items.clear();
		self.rows_to_items.clear();
		self.focus = 0;
	}

	/// Returns the number of items in this table.
	pub fn len(&self) -> usize {
		self.items.len()
	}

	/// Returns `true` if this table has no items.
	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}

	/// Returns the index of the currently selected table row.
	pub fn row(&self) -> Option<usize> {
		if self.items.is_empty() {
			None
		} else {
			Some(self.focus)
		}
	}

	/// Selects the row at the specified index.
	pub fn set_selected_row(&mut self, row_index: usize) {
		self.focus = row_index;
		self.scrollbase.scroll_to(row_index);
	}

	/// Selects the row at the specified index.
	///
	/// Chainable variant.
	pub fn selected_row(self, row_index: usize) -> Self {
		self.with(|t| t.set_selected_row(row_index))
	}

	/// Sets the contained items of the table.
	///
	/// The currently active sort order is preserved and will be applied to all
	/// items.
	pub fn set_items(&mut self, items: Vec<T>) {
		self.items = items;
		self.rows_to_items = Vec::with_capacity(self.items.len());

		for i in 0..self.items.len() {
			self.rows_to_items.push(i);
		}

		if let Some((column, order)) = self.order() {
			self.sort_by(column, order);
		}

		self.scrollbase
			.set_heights(self.last_size.y.saturating_sub(2), self.rows_to_items.len());

		self.set_selected_row(0);
	}

	/// Sets the contained items of the table.
	///
	/// The order of the items will be preserved even when the table is sorted.
	///
	/// Chainable variant.
	pub fn items(self, items: Vec<T>) -> Self {
		self.with(|t| t.set_items(items))
	}

	/// Returns a immutable reference to the item at the specified index
	/// within the underlying storage vector.
	pub fn borrow_item(&mut self, index: usize) -> Option<&T> {
		self.items.get(index)
	}

	/// Returns a mutable reference to the item at the specified index within
	/// the underlying storage vector.
	pub fn borrow_item_mut(&mut self, index: usize) -> Option<&mut T> {
		self.items.get_mut(index)
	}

	/// Returns a immutable reference to the items contained within the table.
	pub fn borrow_items(&mut self) -> &Vec<T> {
		&self.items
	}

	/// Returns a mutable reference to the items contained within the table.
	///
	/// Can be used to modify the items in place.
	pub fn borrow_items_mut(&mut self) -> &mut Vec<T> {
		&mut self.items
	}

	/// Returns the index of the currently selected item within the underlying
	/// storage vector.
	pub fn item(&self) -> Option<usize> {
		if self.items.is_empty() {
			None
		} else {
			Some(self.rows_to_items[self.focus])
		}
	}

	/// Selects the item at the specified index within the underlying storage
	/// vector.
	pub fn set_selected_item(&mut self, item_index: usize) {
		// TODO optimize the performance for very large item lists
		if item_index < self.items.len() {
			for (row, item) in self.rows_to_items.iter().enumerate() {
				if *item == item_index {
					self.focus = row;
					self.scrollbase.scroll_to(row);
					break;
				}
			}
		}
	}

	/// Selects the item at the specified index within the underlying storage
	/// vector.
	///
	/// Chainable variant.
	pub fn selected_item(self, item_index: usize) -> Self {
		self.with(|t| t.set_selected_item(item_index))
	}

	/// Inserts a new item into the table.
	///
	/// The currently active sort order is preserved and will be applied to the
	/// newly inserted item.
	pub fn insert_item(&mut self, item: T) {
		self.items.push(item);
		self.rows_to_items.push(self.items.len());

		self.scrollbase
			.set_heights(self.last_size.y.saturating_sub(2), self.rows_to_items.len());

		if let Some((column, order)) = self.order() {
			self.sort_by(column, order);
		}
	}

	/// Removes the item at the specified index within the underlying storage
	/// vector and returns it.
	pub fn remove_item(&mut self, item_index: usize) -> Option<T> {
		if item_index < self.items.len() {
			// Move the selection if the currently selected item gets removed
			if let Some(selected_index) = self.item() {
				if selected_index == item_index {
					self.focus_up(1);
				}
			}

			// Remove the sorted reference to the item
			self.rows_to_items.retain(|i| *i != item_index);

			// Adjust remaining references
			for ref_index in &mut self.rows_to_items {
				if *ref_index > item_index {
					*ref_index -= 1;
				}
			}

			// Update scroll height to prevent out of index drawing
			self.scrollbase
				.set_heights(self.last_size.y.saturating_sub(2), self.rows_to_items.len());

			// Remove actual item from the underlying storage
			Some(self.items.remove(item_index))
		} else {
			None
		}
	}

	/// Removes all items from the underlying storage and returns them.
	pub fn take_items(&mut self) -> Vec<T> {
		self.scrollbase
			.set_heights(self.last_size.y.saturating_sub(2), 0);
		self.set_selected_row(0);
		self.rows_to_items.clear();
		self.items.drain(0..).collect()
	}
}

impl<T: TableViewItem<H>, H: Eq + Hash + Copy + Clone + 'static> TableView<T, H> {
	fn draw_columns<C: Fn(&Printer, &TableColumn<H>)>(
		&self,
		printer: &Printer,
		sep: &str,
		callback: C,
	) {
		let mut column_offset = 0;
		let column_count = self.columns.len();
		for (index, column) in self.columns.iter().enumerate() {
			let printer = &printer.sub_printer((column_offset, 0), printer.size, true);

			callback(printer, column);

			if index < column_count - 1 {
				printer.print((column.width + 1, 0), sep);
			}

			column_offset += column.width + 3;
		}
	}

	fn sort_items(&mut self, column: H, order: Ordering) {
		if !self.is_empty() {
			let old_item = self.item().unwrap();

			let mut rows_to_items = self.rows_to_items.clone();
			rows_to_items.sort_by(|a, b| {
				if order == Ordering::Less {
					self.items[*a].cmp(&self.items[*b], column)
				} else {
					self.items[*b].cmp(&self.items[*a], column)
				}
			});
			self.rows_to_items = rows_to_items;

			self.set_selected_item(old_item);
		}
	}

	fn draw_item(&self, printer: &Printer, i: usize) {
		self.draw_columns(printer, "┆ ", |printer, column| {
			let value = self.items[self.rows_to_items[i]].to_column(column.column);
			column.draw_row(printer, value.as_str());
		});
	}

	fn focus_up(&mut self, n: usize) {
		self.focus -= cmp::min(self.focus, n);
	}

	fn focus_down(&mut self, n: usize) {
		self.focus = cmp::min(self.focus + n, self.items.len() - 1);
	}

	fn active_column(&self) -> usize {
		self.columns.iter().position(|c| c.selected).unwrap_or(0)
	}

	fn column_cancel(&mut self) {
		self.column_select = false;
		for column in &mut self.columns {
			column.selected = column.order != Ordering::Equal;
		}
	}

	fn column_next(&mut self) -> bool {
		let column = self.active_column();
		if column < self.columns.len() - 1 {
			self.columns[column].selected = false;
			self.columns[column + 1].selected = true;
			true
		} else {
			false
		}
	}

	fn column_prev(&mut self) -> bool {
		let column = self.active_column();
		if column > 0 {
			self.columns[column].selected = false;
			self.columns[column - 1].selected = true;
			true
		} else {
			false
		}
	}

	fn column_select(&mut self) {
		let next = self.active_column();
		let column = self.columns[next].column;
		let current = self
			.columns
			.iter()
			.position(|c| c.order != Ordering::Equal)
			.unwrap_or(0);

		let order = if current != next {
			self.columns[next].default_order
		} else if self.columns[current].order == Ordering::Less {
			Ordering::Greater
		} else {
			Ordering::Less
		};

		self.sort_by(column, order);
	}
}

impl<T: TableViewItem<H> + 'static, H: Eq + Hash + Copy + Clone + 'static> View
	for TableView<T, H>
{
	fn draw(&self, printer: &Printer) {
		self.draw_columns(printer, "╷ ", |printer, column| {
			let color = if column.order != Ordering::Equal || column.selected {
				if self.column_select && column.selected && self.enabled && printer.focused {
					Highlight
				} else {
					HighlightInactive
				}
			} else {
				Primary
			};

			printer.with_color(ColorStyle::from(color), |printer| {
				column.draw_header(printer);
			});
		});

		self.draw_columns(
			&printer.sub_printer((0, 1), printer.size, true),
			"┴─",
			|printer, column| {
				printer.print_hline((0, 0), column.width + 1, "─");
			},
		);

		let printer = &printer.sub_printer((0, 2), printer.size, true);
		self.scrollbase.draw(printer, |printer, i| {
			let color = if i == self.focus {
				if !self.column_select && self.enabled && printer.focused {
					Highlight
				} else {
					HighlightInactive
				}
			} else {
				Primary
			};

			printer.with_color(ColorStyle::from(color), |printer| {
				self.draw_item(printer, i);
			});
		});
	}

	fn layout(&mut self, size: Vec2) {
		if size == self.last_size {
			return;
		}

		let item_count = self.items.len();
		let column_count = self.columns.len();

		// Split up all columns into sized / unsized groups
		let (mut sized, mut usized): (Vec<&mut TableColumn<H>>, Vec<&mut TableColumn<H>>) = self
			.columns
			.iter_mut()
			.partition(|c| c.requested_width.is_some());

		// Subtract one for the separators between our columns (that's column_count - 1)
		let mut available_width = size.x.saturating_sub(column_count.saturating_sub(1) * 3);

		// Reduce the with in case we are displaying a scrollbar
		if size.y.saturating_sub(1) < item_count {
			available_width = available_width.saturating_sub(2);
		}

		// Calculate widths for all requested columns
		let mut remaining_width = available_width;
		for column in &mut sized {
			column.width = match *column.requested_width.as_ref().unwrap() {
				TableColumnWidth::Percent(width) => cmp::min(
					(size.x as f32 / 100.0 * width as f32).ceil() as usize,
					remaining_width,
				),
				TableColumnWidth::Absolute(width) => width,
			};
			remaining_width = remaining_width.saturating_sub(column.width);
		}

		// Spread the remaining with across the unsized columns
		let remaining_columns = usized.len();
		for column in &mut usized {
			column.width = (remaining_width as f32 / remaining_columns as f32).floor() as usize;
		}

		self.scrollbase
			.set_heights(size.y.saturating_sub(2), item_count);
		self.last_size = size;
	}

	fn take_focus(&mut self, _: Direction) -> bool {
		self.enabled && !self.items.is_empty()
	}

	fn on_event(&mut self, event: Event) -> EventResult {
		if !self.enabled {
			return EventResult::Ignored;
		}

		let last_focus = self.focus;
		match event {
			Event::Key(Key::Right) => {
				if self.column_select {
					if !self.column_next() {
						return EventResult::Ignored;
					}
				} else {
					self.column_select = true;
				}
			}
			Event::Key(Key::Left) => {
				if self.column_select {
					if !self.column_prev() {
						return EventResult::Ignored;
					}
				} else {
					self.column_select = true;
				}
			}
			Event::Key(Key::Up) if self.focus > 0 || self.column_select => {
				if self.column_select {
					self.column_cancel();
				} else {
					self.focus_up(1);
				}
			}
			Event::Key(Key::Down) if self.focus + 1 < self.items.len() || self.column_select => {
				if self.column_select {
					self.column_cancel();
				} else {
					self.focus_down(1);
				}
			}
			Event::Key(Key::PageUp) => {
				self.column_cancel();
				self.focus_up(10);
			}
			Event::Key(Key::PageDown) => {
				self.column_cancel();
				self.focus_down(10);
			}
			Event::Key(Key::Home) => {
				self.column_cancel();
				self.focus = 0;
			}
			Event::Key(Key::End) => {
				self.column_cancel();
				self.focus = self.items.len() - 1;
			}
			Event::Key(Key::Enter) => {
				if self.column_select {
					self.column_select();

					if self.on_sort.is_some() {
						let c = &self.columns[self.active_column()];
						let column = c.column;
						let order = c.order;

						let cb = self.on_sort.clone().unwrap();
						return EventResult::Consumed(Some(Callback::from_fn(move |s| {
							cb(s, column, order)
						})));
					}
				} else if !self.is_empty() && self.on_submit.is_some() {
					let cb = self.on_submit.clone().unwrap();
					let row = self.row().unwrap();
					let index = self.item().unwrap();
					return EventResult::Consumed(Some(Callback::from_fn(move |s| {
						cb(s, row, index)
					})));
				}
			}
			_ => return EventResult::Ignored,
		}

		let focus = self.focus;
		self.scrollbase.scroll_to(focus);

		if self.column_select {
			EventResult::Consumed(None)
		} else if !self.is_empty() && last_focus != focus {
			let row = self.row().unwrap();
			let index = self.item().unwrap();
			EventResult::Consumed(
				self.on_select
					.clone()
					.map(|cb| Callback::from_fn(move |s| cb(s, row, index))),
			)
		} else {
			EventResult::Ignored
		}
	}
}

/// A type used for the construction of columns in a
/// [`TableView`](struct.TableView.html).
pub struct TableColumn<H: Copy + Clone + 'static> {
	column: H,
	title: String,
	selected: bool,
	alignment: HAlign,
	order: Ordering,
	width: usize,
	default_order: Ordering,
	requested_width: Option<TableColumnWidth>,
}

enum TableColumnWidth {
	Percent(usize),
	Absolute(usize),
}

impl<H: Copy + Clone + 'static> TableColumn<H> {
	/// Sets the default ordering of the column.
	pub fn ordering(mut self, order: Ordering) -> Self {
		self.default_order = order;
		self
	}

	/// Sets the horizontal text alignment of the column.
	pub fn align(mut self, alignment: HAlign) -> Self {
		self.alignment = alignment;
		self
	}

	/// Sets how many characters of width this column will try to occupy.
	pub fn width(mut self, width: usize) -> Self {
		self.requested_width = Some(TableColumnWidth::Absolute(width));
		self
	}

	/// Sets what percentage of the width of the entire table this column will
	/// try to occupy.
	pub fn width_percent(mut self, width: usize) -> Self {
		self.requested_width = Some(TableColumnWidth::Percent(width));
		self
	}

	fn new(column: H, title: String) -> Self {
		Self {
			column: column,
			title: title,
			selected: false,
			alignment: HAlign::Left,
			order: Ordering::Equal,
			width: 0,
			default_order: Ordering::Less,
			requested_width: None,
		}
	}

	fn draw_header(&self, printer: &Printer) {
		let order = match self.order {
			Ordering::Less => "^",
			Ordering::Greater => "v",
			Ordering::Equal => " ",
		};

		let header = match self.alignment {
			HAlign::Left => format!(
				"{:<width$} [{}]",
				self.title,
				order,
				width = self.width.saturating_sub(4)
			),
			HAlign::Right => format!(
				"{:>width$} [{}]",
				self.title,
				order,
				width = self.width.saturating_sub(4)
			),
			HAlign::Center => format!(
				"{:^width$} [{}]",
				self.title,
				order,
				width = self.width.saturating_sub(4)
			),
		};
		printer.print((0, 0), header.as_str());
	}

	fn draw_row(&self, printer: &Printer, value: &str) {
		let value = match self.alignment {
			HAlign::Left => format!("{:<width$} ", value, width = self.width),
			HAlign::Right => format!("{:>width$} ", value, width = self.width),
			HAlign::Center => format!("{:^width$} ", value, width = self.width),
		};
		printer.print((0, 0), value.as_str());
	}
}
