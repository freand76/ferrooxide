// exit: 1
// Always procedures are not allowed in programs (IEEE 1800-2017 24.3).
program p;
  initial $display("FAILED");
  always #1 $finish;
endprogram
