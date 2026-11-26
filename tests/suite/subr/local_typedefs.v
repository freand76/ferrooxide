// Typedefs declared in a subroutine (IEEE 1800-2017 6.18): visible in it
// only; an enum's constants too.
module top;
  typedef int t_t;
  function automatic int f(int n);
    typedef bit bit_q_t[$];
    typedef enum {P, Q, R} e_t;
    typedef byte t_t;
    bit_q_t q;
    e_t e = Q;
    t_t b = 8'hff;
    for (int i = 0; i < n; i++) q.push_back(i[0]);
    return q.size() + int'(e) + b;
  endfunction
  initial begin
    t_t x = 300;
    if (f(4) == 4 && x == 300) $display("PASSED");
    else $display("FAILED: %0d", f(4));
  end
endmodule
