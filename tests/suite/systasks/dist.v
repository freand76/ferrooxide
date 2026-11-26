// expected
// the $dist_* functions (IEEE 1800-2017 20.15): the values of the algorithms of annex N, and
// the seed variable they update
module top;
  integer s, s0;
  initial begin
    for (s0 = -50; s0 < 50; s0 = s0 + 7) begin
      s = s0 * 104729;
      $write("%0d %0d %0d ", $dist_uniform(s, -10, 100), $dist_normal(s, 5, 100), $dist_exponential(s, 30));
      $display("%0d %0d %0d %0d %0d", $dist_poisson(s, 12), $dist_chi_square(s, 5), $dist_t(s, 3), $dist_erlang(s, 3, 40), s);
    end
  end
endmodule
