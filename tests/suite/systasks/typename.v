// expected
// $typename (IEEE 1800-2017 20.6.1): typedefs resolved, default signing left out, ranges
// as decimal numbers, `$` for an unpacked array
module top #(parameter W = 8);
  typedef bit node;
  typedef logic [W-1:0] word_t;
  node [2:0] x;
  int signed y;
  word_t w;
  logic signed [3:0] sl;
  bit unsigned u;
  reg r;
  wire n;
  integer unsigned iu;
  real re;
  string st;
  byte arr [4];
  initial begin
    $display("%s|%s|%s|%s|%s", $typename(x), $typename(y), $typename(w), $typename(sl), $typename(u));
    $display("%s|%s|%s|%s|%s", $typename(r), $typename(n), $typename(iu), $typename(re), $typename(st));
    $display("%s|%s|%s", $typename(arr), $typename(logic [W:1]), $typename(x + w));
    if ($typename(y) == "int") $display("compared");
  end
endmodule
