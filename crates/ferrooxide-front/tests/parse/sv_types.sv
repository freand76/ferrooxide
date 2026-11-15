package p;
  typedef enum logic [1:0] { A, B = 2, C } e_t;
  typedef struct packed { logic [3:0] hi; e_t lo; } s_t;
  parameter int W = 8;
  function automatic int dbl(int x); return 2 * x; endfunction
endpackage

module top import p::*; #(parameter type T = logic [3:0]) (input logic clk);
  typedef union packed { logic [5:0] v; s_t s; } u_t;
  s_t s;
  u_t u;
  s_t arr [2];
  T tp;
  int i;
  bit [W-1:0] bv;
  e_t e;
  string str;
  always_comb begin
    s = '{hi: 4'h3, lo: B};
    u.s.hi = s.hi;
    arr[1].lo = e;
    i = int'(s) + p::dbl(3);
    bv = 8'(i);
    e = e.next();
    str = "abc";
  end
  always_ff @(posedge clk) begin
    unique case (e) inside
      A, [B:C]: i <= i + 1;
      default: i <= 0;
    endcase
    i++;
    i += 2;
  end
  initial begin
    foreach (arr[k]) arr[k] = '0;
    for (int j = 0; j < $size(arr); j++) begin
      if (j inside {0, 1}) continue;
      break;
    end
    do i--; while (i > 0);
    $display("%0d %0d", $bits(s_t), $left(bv));
  end
endmodule
