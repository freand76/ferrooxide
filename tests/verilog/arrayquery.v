// Array query functions: $left/$right/$low/$high/$size/$increment/$dimensions/
// $unpacked_dimensions, with dimension numbers, parameterized ranges, as loop bounds
module t;
  typedef logic [7:0] byte_t;
  logic [7:0] a;
  logic [0:3][15:8] pk;
  logic [3:0] mem [10:1];
  logic s;
  int i;
  byte_t bt [0:2];
  localparam N = 5;
  logic [N-1:0] pv [N*2:1];
  localparam SZ = $size(pv);
  integer k, total;
  initial begin
    total = 0;
    for (k = $low(pv); k <= $high(pv); k = k + 1) total = total + k;
    $display("pv %0d %0d %0d %0d total %0d SZ %0d", $left(pv), $size(pv, 2), $high(pv, 2), $increment(pv), total, SZ);
    $display("a %0d %0d %0d %0d %0d %0d %0d %0d", $left(a), $right(a), $low(a), $high(a), $size(a), $increment(a), $dimensions(a), $unpacked_dimensions(a));
    $display("pk %0d %0d %0d %0d | %0d %0d %0d %0d", $left(pk), $right(pk), $size(pk), $increment(pk), $left(pk, 2), $right(pk, 2), $size(pk, 2), $dimensions(pk));
    $display("mem %0d %0d %0d %0d %0d | %0d %0d %0d", $left(mem), $right(mem), $size(mem), $increment(mem), $dimensions(mem), $left(mem, 2), $size(mem, 2), $unpacked_dimensions(mem));
    $display("s %0d %0d %0d %0d", $left(s), $right(s), $size(s), $dimensions(s));
    $display("i %0d %0d %0d %0d", $left(i), $right(i), $size(i), $dimensions(i));
    $display("bt %0d %0d %0d %0d", $left(bt), $size(bt), $left(bt, 2), $dimensions(bt));
  end
endmodule
