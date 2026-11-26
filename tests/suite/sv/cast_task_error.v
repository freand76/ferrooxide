// expected
// a failed $cast task is a run-time error (IEEE 1800-2017 6.24.2); the target keeps its value
module top;
  typedef enum {A = 1, B = 2} e_t;
  e_t e;
  initial begin
    e = A;
    $cast(e, 5);
    $display("%0d", e);
    $cast(e, 2);
    $display("%0d", e);
  end
endmodule
