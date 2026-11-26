// randsequence (IEEE 1800-2017 18.17): sequences, weighted alternatives, if/case/repeat,
// rand join, break and return, production arguments
module top;
  int fails = 0;
  string s;
  int n_a, n_b, n_c;

  function automatic string gen(int sel, int stop_at);
    string r = "";
    int i = 0;
    randsequence (main)
      main : head body tail;
      head : { r = {r, "h"}; };
      body : case (sel)
               0 : one;
               1 : two;
               default : many(3, "m");
             endcase;
      one : { r = {r, "1"}; };
      two : repeat (2) one;
      void many(int k, string c = "x") : { repeat (k) r = {r, c}; } { i++; if (i == stop_at) break; } more;
      more : { r = {r, "+"}; if (stop_at == 2) return; r = {r, "!"}; };
      tail : if (sel > 5) { r = {r, "T"}; } else { r = {r, "t"}; };
    endsequence
    return r;
  endfunction

  initial begin
    if (gen(0, 0) != "h1t") fails++;
    if (gen(1, 0) != "h11t") fails++;
    if (gen(7, 0) != "hmmm+!T") fails++;
    // break ends the whole randsequence
    if (gen(3, 1) != "hmmm") fails++;
    // return ends the production
    if (gen(3, 2) != "hmmm+t") fails++;
    // weights: one alternative never, the others in proportion
    n_a = 0; n_b = 0; n_c = 0;
    repeat (3000)
      randsequence ()
        pick : a := 3 | b := 0 | c;
        a : { n_a++; };
        b : { n_b++; };
        c : { n_c++; };
      endsequence
    if (n_b != 0 || n_a + n_c != 3000 || n_a < 2000 || n_a > 2500) fails++;
    // rand join: every item once, each production's items in order
    repeat (200) begin
      s = "";
      randsequence (j)
        j : rand join (0.5) x y;
        x : { s = {s, "a"}; } { s = {s, "b"}; } { s = {s, "c"}; };
        y : { s = {s, "1"}; } { #1 s = {s, "2"}; };
      endsequence
      begin
        int pa = -1, pb = -1, pc = -1, p1 = -1, p2 = -1;
        for (int k = 0; k < s.len(); k++)
          case (s[k])
            "a": pa = k;
            "b": pb = k;
            "c": pc = k;
            "1": p1 = k;
            "2": p2 = k;
          endcase
        if (s.len() != 5 || !(pa < pb && pb < pc && p1 < p2) || pa < 0 || p1 < 0) fails++;
      end
    end
    if (fails == 0)
      $display("PASSED");
    else
      $display("FAILED %0d (%0d %0d %0d)", fails, n_a, n_b, n_c);
  end
endmodule
