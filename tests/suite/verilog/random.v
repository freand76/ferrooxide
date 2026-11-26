// expected
// $random gives the sequence of the standard's algorithm (IEEE 1364-2005 17.9.1); the
// values of $urandom and $urandom_range (seeded and unseeded) are this simulator's (IEEE
// 1800-2017 18.13 leaves them open): the same from run to run, a process's own (18.14)
module top;
  integer s, i;
  initial begin
    s = 5;
    for (i = 0; i < 4; i = i + 1) $display("%0d %0d", $random(s), s);
    for (i = 0; i < 4; i = i + 1) $display("r %0d", $random);
    for (i = 0; i < 3; i = i + 1) $display("u %0d", $urandom);
    s = 7;
    for (i = 0; i < 3; i = i + 1) $display("us %0d %0d", $urandom(s), s);
    for (i = 0; i < 3; i = i + 1) $display("ur %0d", $urandom_range(10, 3));
  end
endmodule
module top2;
  integer i;
  initial for (i = 0; i < 4; i = i + 1) $display("%0d %0d %0d", $urandom_range(10, 3), $urandom_range(100), $urandom_range(0, 7));
endmodule
