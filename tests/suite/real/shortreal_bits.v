// $shortrealtobits and $bitstoshortreal (IEEE 1800-2017 20.5): IEEE 754 single precision
module top;
  shortreal s;
  int fails = 0;
  initial begin
    if ($shortrealtobits(1.0) !== 32'h3f800000) fails++;
    if ($shortrealtobits(-2.5) !== 32'hc0200000) fails++;
    if ($bitstoshortreal(32'h40490fdb) < 3.14159 || $bitstoshortreal(32'h40490fdb) > 3.1416) fails++;
    s = $bitstoshortreal($shortrealtobits(12.45));
    if (!(s > 12.449 && s < 12.451)) fails++;
    // (the value has single precision)
    if ($bitstoshortreal($shortrealtobits(0.1)) == 0.1) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
