// exit: 1
// An automatic variable cannot be written by a nonblocking assignment (a member) (IEEE 1800-2017 6.21).
module test;
  reg g;
  task automatic t;
    reg v;
    struct packed { logic a; logic b; } s;
    v = 0;
    s.a <= 1;
  endtask
  initial t;
endmodule
