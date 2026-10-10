//! The default units sheet: SI + common imperial, as ordinary editable cells.

use crate::model::{Sheet, Workbook};

pub const UNITS: &[(&str, &str)] = &[
    ("dim length", "SI base dimensions"),
    ("dim mass", ""),
    ("dim time", ""),
    ("dim current", ""),
    ("dim temperature", ""),
    ("dim amount", ""),
    ("dim luminosity", ""),
    ("base [m] length", "SI base units"),
    ("base [kg] mass", ""),
    ("base [s] time", ""),
    ("base [A] current", ""),
    ("base [K] temperature", ""),
    ("base [mol] amount", ""),
    ("base [cd] luminosity", ""),
    ("[km] = 1000 [m]", "length"),
    ("[cm] = 0.01 [m]", ""),
    ("[mm] = 0.001 [m]", ""),
    ("[µm] = 0.000001 [m]", ""),
    ("[in] = 0.0254 [m]", ""),
    ("[ft] = 12 [in]", ""),
    ("[yd] = 3 [ft]", ""),
    ("[mi] = 1609.344 [m]", ""),
    ("[nmi] = 1852 [m]", ""),
    ("[g] = 0.001 [kg]", "mass"),
    ("[mg] = 0.001 [g]", ""),
    ("[t] = 1000 [kg]", ""),
    ("[lb] = 0.45359237 [kg]", ""),
    ("[oz] = 1 16 / [lb]", ""),
    ("[ms] = 0.001 [s]", "time"),
    ("[min] = 60 [s]", ""),
    ("[h] = 60 [min]", ""),
    ("[day] = 24 [h]", ""),
    ("[week] = 7 [day]", ""),
    ("[yr] = 365.25 [day]", ""),
    ("[month] = 1 12 / [yr]", ""),
    ("[date] = 1 [day] offset 0", "absolute: days since 1970-01-01"),
    ("[Δ°C] = 1 [K]", "temperature differences"),
    ("[Δ°F] = 5 9 / [K]", ""),
    ("[°C] = 1 [Δ°C] offset 273.15", "absolute temperatures (input/display only)"),
    ("[°F] = 1 [Δ°F] offset 255.37222222222223", ""),
    ("[ha] = 10000 [m^2]", "area / volume"),
    ("[acre] = 4046.8564224 [m^2]", ""),
    ("[L] = 0.001 [m^3]", ""),
    ("[mL] = 0.001 [L]", ""),
    ("[gal] = 3.785411784 [L]", ""),
    ("[Hz] = 1 [1/s]", "derived"),
    ("[N] = 1 [kg*m/s^2]", ""),
    ("[J] = 1 [N*m]", ""),
    ("[kJ] = 1000 [J]", ""),
    ("[W] = 1 [J/s]", ""),
    ("[kW] = 1000 [W]", ""),
    ("[MW] = 1000 [kW]", ""),
    ("[Wh] = 1 [W*h]", ""),
    ("[kWh] = 1000 [Wh]", ""),
    ("[Pa] = 1 [N/m^2]", ""),
    ("[kPa] = 1000 [Pa]", ""),
    ("[bar] = 100000 [Pa]", ""),
    ("[psi] = 6894.757293168 [Pa]", ""),
    ("[V] = 1 [W/A]", ""),
    ("[Ω] = 1 [V/A]", ""),
    ("[C] = 1 [A*s]", ""),
    ("[kph] = 1 [km/h]", ""),
    ("[mph] = 1 [mi/h]", ""),
    ("[cal] = 4.184 [J]", ""),
    ("[kcal] = 1000 [cal]", ""),
    ("[%] = 0.01", "dimensionless"),
    ("dim currency", "money: rates are ordinary inputs, scrub them"),
    ("base [USD] currency", ""),
    ("[EUR] = 1.08 [USD]", ""),
    ("[GBP] = 1.27 [USD]", ""),
    ("[JPY] = 0.0067 [USD]", ""),
];

pub fn units_sheet() -> Sheet {
    let mut s = Sheet::new("units");
    for (i, (decl, note)) in UNITS.iter().enumerate() {
        let k = s.key(i, 0).unwrap();
        s.cells.insert((k.row, k.col), crate::model::Cell::new(vec![crate::model::Piece::Text(decl.to_string())]));
        if !note.is_empty() {
            let k = s.key(i, 1).unwrap();
            s.cells.insert((k.row, k.col), crate::model::Cell::new(vec![crate::model::Piece::Text(note.to_string())]));
        }
    }
    s.col_widths.insert(s.cols.get(0).unwrap(), 230.0);
    s.col_widths.insert(s.cols.get(1).unwrap(), 300.0);
    s
}

pub fn default_workbook() -> Workbook {
    let mut wb = Workbook::empty();
    wb.sheets.push(Sheet::new("Sheet1"));
    wb.sheets.push(units_sheet());
    wb
}
