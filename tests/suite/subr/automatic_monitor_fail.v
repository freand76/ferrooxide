// exit: 1
// An automatic variable cannot be an argument of $monitor or $strobe (read after it is gone) (IEEE 1800-2017 6.21).
module test;
  reg g;
  task automatic t;
    reg v;
    struct packed { logic a; logic b; } s;
    v = 0;
    $monitor("%0d", v);
  endtask
  initial t;
endmodule
