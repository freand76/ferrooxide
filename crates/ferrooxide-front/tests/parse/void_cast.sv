module m;
  int q[$];
  function int f(int x);
    return x;
  endfunction
  initial begin
    void'(f(1));
    void'(q.pop_back());
    void'($random);
  end
endmodule
