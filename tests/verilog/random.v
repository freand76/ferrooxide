// $random, $urandom and $urandom_range give Icarus's sequences (seeded and unseeded)
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
