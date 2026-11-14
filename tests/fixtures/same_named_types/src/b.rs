pub struct Iter { n: usize }
impl Iter {
    fn err(&self) -> usize { self.n } // dead_b_err
    fn parse_line(&self) -> usize { self.err() } // dead_b_parse
    fn amb(&self) -> usize { 2 } // amb_b
}
