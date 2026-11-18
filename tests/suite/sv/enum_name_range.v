// Enum name ranges (IEEE 1800-2017 6.19): `N[3]` declares N0, N1, N2 and `M[5:3]` M5, M4, M3,
// numbered on from the previous name or the given first value.
module test;
  enum { A = 10, N[3], M[5:3] = 20, L } e;
  enum logic [3:0] { R[2:3] = 4'd1, S[1] } r;
  integer fails = 0;

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  initial begin
    check(N0, 11, `__LINE__);
    check(N2, 13, `__LINE__);
    check(M5, 20, `__LINE__);
    check(M4, 21, `__LINE__);
    check(M3, 22, `__LINE__);
    check(L, 23, `__LINE__);
    check(R2, 1, `__LINE__);
    check(R3, 2, `__LINE__);
    check(S0, 3, `__LINE__);
    e = e.first;
    e = e.next(2);
    check(e, N1, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
