// exit: 1
// A constraint on the elements of a rand dynamic array is not supported yet: rejected,
// not ignored.
class C;
  rand byte d[];
  constraint k { d.size() == 2; foreach (d[i]) d[i] < 5; }
endclass
module t;
  C c = new;
  initial void'(c.randomize());
endmodule
