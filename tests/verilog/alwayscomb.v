// always_comb / always_latch are sensitive to what called functions read (also void functions
// called as statements and recursive automatic functions); always @* is not
module t;
  logic [7:0] value;
  logic hidden, en, y_comb, y_star, q;
  integer ones, counter;

  function f(input a);
    f = a ^ hidden;
  endfunction

  function automatic integer cnt_a(input integer k);
    cnt_a = value[k] + (k > 0 ? cnt_b(k - 1) : 0);
  endfunction
  function automatic integer cnt_b(input integer k);
    cnt_b = value[k] + (k > 0 ? cnt_a(k - 1) : 0);
  endfunction

  function automatic void count();
    counter = 0;
    for (integer i = 0; i < 8; i++) counter += value[i];
  endfunction

  always_comb y_comb = f(1'b0);
  always @* y_star = f(1'b0);
  always_comb ones = cnt_a(7);
  always_comb count();
  always_latch if (en) q = f(value[0]);

  initial begin
    hidden = 0;
    value = 8'h0F;
    en = 1;
    #1 $display("y_comb %b y_star %b ones %0d counter %0d q %b", y_comb, y_star, ones, counter, q);
    hidden = 1;
    #1 $display("y_comb %b y_star %b ones %0d counter %0d q %b", y_comb, y_star, ones, counter, q);
    value = 8'hF3;
    #1 $display("y_comb %b y_star %b ones %0d counter %0d q %b", y_comb, y_star, ones, counter, q);
    en = 0;
    hidden = 0;
    #1 $display("y_comb %b y_star %b ones %0d counter %0d q %b", y_comb, y_star, ones, counter, q);
  end
endmodule
