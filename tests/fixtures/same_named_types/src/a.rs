pub struct Iter { n: usize }
impl Iter {
    fn err(&self) -> usize { self.n }
    fn parse_line(&self) -> usize { self.err() }
    fn amb(&self) -> usize { 1 } // amb_a
    pub fn run(&self) -> usize { self.parse_line() }
}
