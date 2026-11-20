// exit: 1
// An automatic variable cannot be read by a procedural continuous assignment (IEEE 1800-2017 6.21).
module test;
  reg g;
  task automatic t;
    reg v;
    struct packed { logic a; logic b; } s;
    v = 0;
    force g = v;
  endtask
  initial t;
endmodule
